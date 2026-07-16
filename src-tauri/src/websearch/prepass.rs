//! The classifier: stage two of the search decision, for the ambiguous middle
//! the deterministic [`super::prefilter`] could not resolve.
//!
//! One grammar-constrained LLM call decides whether the web is needed and, if
//! so, rewrites the (possibly context-dependent) message into a standalone
//! question plus 1-3 keyword queries. The model is constrained by a strict
//! `response_format` JSON schema, verified to coexist with the engine's
//! reasoning-control flow, so even small local models emit a parseable shape.
//!
//! ## Prompt shape (persona-free by design)
//!
//! The classifier runs under its **own** short system prompt, NOT the chat
//! persona. This is deliberate: the chat persona instructs the model how to
//! behave toward the user (including, historically, to deflect current-info
//! questions), which biases a decision made inside that context. Decoupling the
//! decision from the persona is the correctness fix at the heart of this
//! module's redesign. A few-shot header and an explicit "when unsure of your own
//! freshness, choose web" rule bias the small local models toward searching
//! rather than answering from stale memory. Only the last few conversation turns
//! are embedded, as plain text for pronoun resolution, so a follow-up like "what
//! about there?" can still be rewritten into a standalone query.
//!
//! Dropping the persona prefix costs a small extra prefill on the ambiguous
//! turns that reach this stage (the engine runs `--parallel 1`), traded
//! knowingly for a correct decision. The pre-filter already resolves the obvious
//! turns with no model call at all, so this stage fires far less often than a
//! per-message pre-pass would.
//!
//! ## Failure policy
//!
//! A malformed or unparseable response degrades to [`SearchDecision::No`]
//! (answer directly) rather than a spurious search: a false negative is cheap
//! and recoverable, whereas a false positive spends latency and a third-party
//! request on nothing.

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::commands::ChatMessage;

/// The three-way search decision emitted by the pre-pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchDecision {
    /// Answer directly from the model; no retrieval.
    No,
    /// Answer from source blocks already fetched earlier in this conversation.
    Cached,
    /// Run the retrieval pipeline.
    Web,
}

/// Which retrieval tier the classifier judged best for this turn. Advisory only:
/// the orchestrator combines it with deterministic gates (a vertical may still
/// run on its own signal, and the wiki tier is additionally volatility-guarded),
/// and an unknown or missing route parses to [`SearchRoute::Web`] (the general
/// engine tier), never a panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchRoute {
    /// Current weather or forecast for a location → weather vertical.
    Weather,
    /// Current events, recent developments, general news → news vertical.
    News,
    /// Stable definitional or historical facts → Wikipedia vertical.
    Wiki,
    /// Live scores, fixtures, or standings for a named competition/team →
    /// sports vertical. Advisory: the orchestrator also runs the sports
    /// vertical on its own deterministic league-keyword signal regardless of
    /// this route (see [`crate::websearch::sports::detect_league`]).
    Sports,
    /// Everything else (software versions, prices, niche live facts) → engines.
    Web,
}

impl SearchRoute {
    /// Normalises the raw `route` string from the model to a [`SearchRoute`].
    /// Any value that is not one of the five known tiers (including an empty or
    /// missing field) maps to [`SearchRoute::Web`], so a malformed route never
    /// fails the turn and only ever falls back to the general engine tier.
    pub(crate) fn from_wire(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "weather" => SearchRoute::Weather,
            "news" => SearchRoute::News,
            "wiki" => SearchRoute::Wiki,
            "sports" => SearchRoute::Sports,
            _ => SearchRoute::Web,
        }
    }
}

/// The pre-pass result: the decision, the routing hint, and the rewritten
/// question and queries used by the retrieval stages when the decision is
/// `Cached` or `Web`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrePassDecision {
    pub decision: SearchDecision,
    pub route: SearchRoute,
    pub standalone_question: String,
    pub queries: Vec<String>,
    /// The user explicitly asked us to look it up / search / verify / double-
    /// check (Anthropic search-trigger category (d)). Signals that whatever
    /// source answered the prior turn was insufficient, so the orchestrator
    /// must skip the multi-turn cache AND every vertical fast path and answer
    /// only from the scraped engines (see [`crate::websearch::orchestrator`]).
    /// Defaults to `false`; set `true` only for a genuine look-it-up request.
    pub explicit_search: bool,
    /// The ISO 639-1 code of the language the USER wrote their latest message
    /// in, as named by the model, or `""` when the model named none.
    ///
    /// A judgement about the ORIGINAL message, deliberately not about the
    /// rewritten question: the classifier is free to hedge a rewrite toward the
    /// English corpus (measured: it spontaneously emits an English companion
    /// query beside the native one), so the rewrite's own wording says nothing
    /// about what the user wrote. This field is what a deterministic character
    /// rule cannot see: Vietnamese carrying no distinctive diacritic at all
    /// ("giá vàng hôm nay bao nhiêu").
    ///
    /// UNTRUSTED even though the grammar enum-constrains it: it is validated
    /// against the static allowlist in
    /// [`crate::websearch::lang::resolve_lang`] before it can influence any
    /// outbound request, so nothing but a `&'static str` from that table ever
    /// reaches a URL or a hostname.
    pub lang: String,
}

/// Why a pre-pass inference call failed at the transport level. Distinct from a
/// merely unparseable body, which is handled in-band by degrading to `No`.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum InferenceError {
    /// The engine request failed (connect, HTTP, read).
    #[error("inference request failed: {0}")]
    Request(String),
    /// The request was cancelled (new turn or user cancel).
    #[error("cancelled")]
    Cancelled,
}

/// Upper bound on keyword queries the pre-pass may emit, matching the schema
/// and the downstream fan-out cap.
const MAX_QUERIES: usize = 3;

/// Injectable pre-pass inference. The orchestrator depends on this trait so its
/// branch logic is tested with [`FakePrePass`]; the builtin engine backing lives
/// in the coverage-excluded [`BuiltinPrePass`].
#[async_trait]
pub trait PrePass: Send + Sync {
    /// Decides search intent for `latest_user_message` given the conversation
    /// so far, under the classifier's own persona-free prompt.
    ///
    /// `latest_images` is base64 image payloads for the latest user turn when
    /// the loaded model is vision-capable. Empty/`None` keeps the text-only
    /// path (no multimodal prefill). Never returns a bad-JSON error: an
    /// unparseable model response degrades in-band to [`SearchDecision::No`].
    async fn decide(
        &self,
        history: &[ChatMessage],
        latest_user_message: &str,
        latest_images: Option<&[String]>,
        today: &str,
        cancel: &CancellationToken,
    ) -> Result<PrePassDecision, InferenceError>;
}

/// The production [`PrePass`], backed by the bundled `llama-server` engine over
/// its OpenAI-compatible `/v1` endpoint. Excluded from the coverage gate: it is
/// thin glue over [`crate::openai::request_openai_json`] and the pure helpers
/// ([`build_prepass_messages`], [`prepass_schema`], [`parse_prepass`],
/// [`prepass_or_no`]), which are all tested directly.
pub struct BuiltinPrePass {
    client: reqwest::Client,
    /// Engine base URL, e.g. `http://127.0.0.1:<port>`.
    base_url: String,
    /// Installed model id resolved from the manifest.
    model: String,
    /// Per-call wall-clock timeout (seconds).
    timeout_secs: u64,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl BuiltinPrePass {
    /// Builds a pre-pass bound to a llama-server `/v1` endpoint (`base_url`), the
    /// installed `model`, and a per-call wall-clock `timeout_secs`.
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
impl PrePass for BuiltinPrePass {
    async fn decide(
        &self,
        history: &[ChatMessage],
        latest_user_message: &str,
        latest_images: Option<&[String]>,
        today: &str,
        cancel: &CancellationToken,
    ) -> Result<PrePassDecision, InferenceError> {
        let messages = build_prepass_messages(history, latest_user_message, latest_images, today);
        let raw = crate::openai::request_openai_json(
            &self.base_url,
            &self.model,
            &self.client,
            messages,
            prepass_schema(),
            None,
            self.timeout_secs,
            crate::config::defaults::PREPASS_MAX_TOKENS,
            crate::openai::V1Flavor::Builtin,
            cancel,
        )
        .await;
        match raw {
            // A 2xx response with unparseable JSON degrades to `No` in-band.
            Ok(content) => Ok(prepass_or_no(parse_prepass(&content), latest_user_message)),
            Err(crate::openai::OpenAiError::Cancelled) => Err(InferenceError::Cancelled),
            Err(other) => Err(InferenceError::Request(format!("{other:?}"))),
        }
    }
}

/// The classifier's own system prompt: a short, persona-free routing role with a
/// few-shot header and an explicit bias toward searching when the model cannot
/// vouch for its own freshness. Kept separate from the chat persona on purpose
/// (see module docs): the persona must not colour the decision.
///
/// The leading `Reasoning: low` line is the gpt-oss (harmony) reasoning-effort
/// directive: without it the model spends 1000+ chain-of-thought tokens on a
/// three-way classification and can blow the call timeout (observed live at
/// ~63 tok/s decode). Inert plain text for every other model family.
const CLASSIFIER_SYSTEM: &str = "Reasoning: low\n\nYou are a retrieval-routing classifier inside a local AI assistant. Your only job is to decide whether answering the user's latest message needs a fresh web search, to pick which source best answers it, and if so to rewrite it into a standalone search query. You never answer the message itself.\n\nOutput ONLY a JSON object: {\"search\": \"no\"|\"cached\"|\"web\", \"route\": \"weather\"|\"news\"|\"wiki\"|\"sports\"|\"web\", \"standalone_question\": \"...\", \"queries\": [\"...\"], \"explicit_search\": true|false, \"lang\": \"<ISO 639-1 code>\"}.\n\nChoose \"search\":\n- \"web\" when a good answer needs any of: (a) recent events, news, or announcements; (b) current prices, rates, scores, weather, or statistics; (c) a fact about a specific person, organization, or product that can change after your training cutoff, such as an age, title, role, employer, team, marital status, ownership, net worth, or current status; (d) an explicit request to search or verify; or any release, version, schedule, or other live fact. A present-tense attribute of a person or entity (\"how old is X now\", \"is Z still married\") is a \"web\" turn even with no freshness word: your training is frozen and the date you are given does not refresh what you remember, unless you already searched this person or entity earlier in this same conversation, in which case choose \"cached\": the fetched sources carry stable biographical facts like a birth date, and the reuse path verifies before answering.\n- \"cached\" when this message repeats or rephrases a question the assistant already searched earlier in this same conversation, OR asks about a detail of a person, entity, or topic the assistant already searched, where those earlier sources plausibly carry the answer (a bio or profile page carries an age, an employer, or family; a product page carries specs and a price tier). A live or volatile detail is NEVER cached, even as a follow-up: a current score, a kickoff time still ahead, a market price, a net worth, today's weather, or breaking news is always \"web\" with a refined standalone question, because those change faster than the earlier sources did.\n- \"no\" only for a stable answer you can give confidently: an established or historical fact, math, a science or coding fundamental, a creative or text-transform task, analysis of text already provided, or a greeting or conversational turn.\nWhen you are unsure whether your knowledge is up to date, choose \"web\": a needless search is far cheaper than a confidently wrong answer.\n\nChoose \"route\" (which source best answers it):\n- \"weather\" for current weather or forecast for a place.\n- \"news\" for current events, elections, and anything asking the latest, current, or recent state of an evolving topic (a conflict, a company, a policy) that is not a live score, fixture, or standings.\n- \"wiki\" for stable definitional or historical facts that do not change from month to month.\n- \"sports\" for live scores, fixtures, or standings for a named competition or team, or the status of an ongoing match or tournament.\n- \"web\" for everything else (software versions, prices, product specs, niche live facts).\nWhen a question is about the present state of an ongoing event, route \"news\" (or \"sports\" for a score/fixture/standings question), never \"wiki\", even if it is phrased like \"what is ...\". Always set a route, even when search is \"no\".\n\n\"standalone_question\": the latest message rewritten as one self-contained question, resolving pronouns and references from the conversation, including entities named in the assistant's previous answers, not only in the user's questions. When the follow-up is an ellipsis like \"how about X?\" or \"what about X?\", keep the SAME question the conversation was already asking and swap in only the new subject X; do not invent a different kind of question.\n\"queries\": 1 to 3 short keyword search queries, not full sentences. When the question is quantitative or ambiguous about which number is meant (a rate versus a level or total, a count versus a share, growth versus size, an age versus a birth date, \"GDP\" / \"how much\" / \"amount\" / \"worth\" / \"size\" without saying growth or rate), emit DISTINCT queries that cover the different answer shapes rather than near-synonym restates of one shape. At least one query MUST target a level/total/amount/USD/size shape (e.g. \"nominal GDP USD\", \"net worth\", \"population total\"), and put that level-shaped query FIRST when the user did not explicitly ask only for growth or rate. Example: \"what's Vietnam's latest GDP\" -> queries [\"Vietnam nominal GDP USD\", \"Vietnam GDP 2026\"] not two growth-only paraphrases.\n\"explicit_search\": true ONLY when the user explicitly asks you to look it up, search, verify, double-check, or confirm (\"can you look it up\", \"search for it\", \"double-check that\"); otherwise false. When true, also set \"search\":\"web\" and put the FULL topic being looked up into the standalone_question, resolved from the conversation, never the literal words \"look it up\".\n\nExamples (message -> JSON):\n\"who is the CEO of OpenAI right now\" -> {\"search\":\"web\",\"route\":\"web\",\"standalone_question\":\"who is the current CEO of OpenAI\",\"queries\":[\"openai ceo\"]}\n\"Vietnam latest GDP\" -> {\"search\":\"web\",\"route\":\"web\",\"standalone_question\":\"what is Vietnam's latest GDP\",\"queries\":[\"Vietnam nominal GDP USD\",\"Vietnam GDP 2026\"],\"explicit_search\":false}\n\"what is the boiling point of water\" -> {\"search\":\"no\",\"route\":\"wiki\",\"standalone_question\":\"what is the boiling point of water\",\"queries\":[\"boiling point of water\"]}\n\"what is photosynthesis\" -> {\"search\":\"web\",\"route\":\"wiki\",\"standalone_question\":\"what is photosynthesis\",\"queries\":[\"photosynthesis\"]}\n\"weather in Paris\" -> {\"search\":\"web\",\"route\":\"weather\",\"standalone_question\":\"what is the current weather in Paris\",\"queries\":[\"paris weather\"]}\n\"what's the latest status of the World Cup 2026\" -> {\"search\":\"web\",\"route\":\"news\",\"standalone_question\":\"what is the current status of the 2026 World Cup\",\"queries\":[\"world cup 2026 status\"]}\n\"who won the most recent F1 race\" -> {\"search\":\"web\",\"route\":\"news\",\"standalone_question\":\"who won the most recent Formula 1 race\",\"queries\":[\"latest f1 race winner\"]}\n\"what's the score of the Lakers game\" -> {\"search\":\"web\",\"route\":\"sports\",\"standalone_question\":\"what is the current score of the Los Angeles Lakers game\",\"queries\":[\"lakers score\"]}\n(you already searched and answered \"what's the latest stable Rust version\" with web sources earlier in this conversation) \"what's the latest stable Rust version\" -> {\"search\":\"cached\",\"route\":\"web\",\"standalone_question\":\"what is the latest stable Rust version\",\"queries\":[\"rust latest stable version\"]}\n\"write a short poem about autumn\" -> {\"search\":\"no\",\"route\":\"web\",\"standalone_question\":\"write a short poem about autumn\",\"queries\":[\"autumn poem\"]}\n(after discussing France) \"and its population?\" -> {\"search\":\"no\",\"route\":\"wiki\",\"standalone_question\":\"what is the population of France\",\"queries\":[\"france population\"]}\n(after discussing the US president) \"what about Argentina?\" -> {\"search\":\"web\",\"route\":\"web\",\"standalone_question\":\"who is the current president of Argentina\",\"queries\":[\"argentina president\"]}\n(you just told the user Elon Musk's net worth is about $240 billion) \"How about Donald Trump?\" -> {\"search\":\"web\",\"route\":\"web\",\"standalone_question\":\"what is Donald Trump's net worth\",\"queries\":[\"donald trump net worth\"]}\n(your previous answer said Jensen Huang is the CEO of Nvidia) \"how much is he worth?\" -> {\"search\":\"web\",\"route\":\"web\",\"standalone_question\":\"what is Jensen Huang's net worth\",\"queries\":[\"jensen huang net worth\"]}\n(you already searched Elon Musk's net worth this conversation and answered from a profile page) \"and how old is he now?\" -> {\"search\":\"cached\",\"route\":\"web\",\"standalone_question\":\"how old is Elon Musk\",\"queries\":[\"elon musk age\"],\"explicit_search\":false}\n\"how old is the Pope\" -> {\"search\":\"web\",\"route\":\"web\",\"standalone_question\":\"how old is the current Pope\",\"queries\":[\"pope age\"],\"explicit_search\":false}\n(you told the user the Belgium vs Spain 2026 World Cup match is today but the scoreboard carried no kickoff time) \"can you look it up please?\" -> {\"search\":\"web\",\"route\":\"sports\",\"standalone_question\":\"what time is the Belgium vs Spain 2026 World Cup match today\",\"queries\":[\"belgium spain world cup kickoff time\"],\"explicit_search\":true}\n(you just gave the World Cup match's final score) \"and at what exact time did it kick off?\" -> {\"search\":\"web\",\"route\":\"sports\",\"standalone_question\":\"what time did the Belgium vs Spain World Cup match kick off\",\"queries\":[\"belgium spain world cup kickoff time\"],\"explicit_search\":false}\n\nLanguage:\nWrite \"standalone_question\" and every entry of \"queries\" in the SAME language the user wrote their latest message in. Never translate them into English: a Vietnamese question searches the Vietnamese web, a Japanese question the Japanese web, and an English query would retrieve the wrong sources for it. You may ADD one English query alongside the native one when you judge the English web would also help; keep the native query first.\nSet \"lang\" to the ISO 639-1 code of the language the USER wrote in, one of: en, vi, ja, zh, ko, th, ar, es, fr, de, pt, ru, hi, id. Judge the language of the user's own words, not of any name or loanword inside them: \"what does pho mean\" is English. If the user wrote in a language not on that list, use the closest one on it, and otherwise \"en\".\nThis changes NOTHING about the \"search\" or \"route\" decision: decide both exactly as above, in exactly the same way you would for the same question asked in English.\n\nExamples (message -> JSON):\n\"thời tiết Hà Nội hôm nay thế nào\" -> {\"search\":\"web\",\"route\":\"weather\",\"standalone_question\":\"thời tiết Hà Nội hôm nay thế nào\",\"queries\":[\"thời tiết Hà Nội hôm nay\"],\"explicit_search\":false,\"lang\":\"vi\"}\n\"giá vàng hôm nay bao nhiêu\" -> {\"search\":\"web\",\"route\":\"web\",\"standalone_question\":\"giá vàng hôm nay bao nhiêu\",\"queries\":[\"giá vàng hôm nay\"],\"explicit_search\":false,\"lang\":\"vi\"}\n\"東京の今日の天気は\" -> {\"search\":\"web\",\"route\":\"weather\",\"standalone_question\":\"東京の今日の天気は\",\"queries\":[\"東京 天気 今日\"],\"explicit_search\":false,\"lang\":\"ja\"}\n\"what does phở mean\" -> {\"search\":\"no\",\"route\":\"wiki\",\"standalone_question\":\"what does phở mean\",\"queries\":[\"phở meaning\"],\"explicit_search\":false,\"lang\":\"en\"}\n\"who is the CEO of OpenAI right now\" -> {\"search\":\"web\",\"route\":\"web\",\"standalone_question\":\"who is the current CEO of OpenAI\",\"queries\":[\"openai ceo\"],\"explicit_search\":false,\"lang\":\"en\"}";

/// The trailing instruction on the classifier's user turn, after the optional
/// conversation block and the latest message.
const CLASSIFIER_INSTRUCTION: &str =
    "Decide for the latest message and output only the JSON object.";

/// Extra instruction appended when the latest turn carries vision images so the
/// classifier uses pixels for routing without inventing unseen brand names.
const CLASSIFIER_VISION_INSTRUCTION: &str = "Attached image(s) are part of the latest message: use what you see for the search decision and query rewrite. Choose search \"no\" for pure visual identification or description (what is this, what logo is this, describe this, what color, read this text) when the image alone answers and the user is not asking for live prices, news, scores, current status, or other web-fresh facts about the entity. Choose \"web\" only when the user needs facts beyond what the image shows. When searching, put only entities and text you can actually read or identify into queries; do not invent brand names or OCR text you cannot see.";

/// Header introducing the embedded conversation context in the classifier's user
/// turn. The turns are context for pronoun resolution only, never instructions.
const CONVERSATION_HEADER: &str = "Conversation so far (context only):";

/// Builds the `response_format` JSON schema constraining the pre-pass output.
///
/// `lang` is enum-constrained to the language allowlist
/// ([`crate::websearch::lang::supported_langs`], the same table every outbound
/// request shape is read from), so the grammar itself makes an out-of-range code
/// impossible at the source rather than only rejecting it downstream. It is
/// `required`: the field is the only signal that can name a language a character
/// rule cannot see, and a model left free to omit it would omit it on exactly the
/// turns it matters.
pub(crate) fn prepass_schema() -> serde_json::Value {
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
                "maxItems": MAX_QUERIES
            },
            "explicit_search": { "type": "boolean" },
            "lang": {
                "type": "string",
                "enum": crate::websearch::lang::supported_langs()
            }
        },
        "required": [
            "search",
            "route",
            "standalone_question",
            "queries",
            "explicit_search",
            "lang"
        ],
        "additionalProperties": false
    })
}

/// Assembles the classifier message array: the classifier's own system prompt,
/// then a single user turn that embeds the last few conversation turns (for
/// pronoun resolution), the latest message, today's date, and the output
/// instruction. The chat persona is intentionally absent (see module docs).
///
/// When `latest_images` is non-empty, those base64 payloads attach to the user
/// message so a vision model can route from pixels. Text-only turns pass
/// `None` and keep the prior wire shape bit-identical.
pub(crate) fn build_prepass_messages(
    history: &[ChatMessage],
    latest_user_message: &str,
    latest_images: Option<&[String]>,
    today: &str,
) -> Vec<ChatMessage> {
    let has_images = latest_images.is_some_and(|imgs| !imgs.is_empty());
    let content = build_classifier_user_turn(history, latest_user_message, today, has_images);
    let images = if has_images {
        latest_images.map(|imgs| imgs.to_vec())
    } else {
        None
    };
    vec![
        ChatMessage {
            role: "system".to_string(),
            content: CLASSIFIER_SYSTEM.to_string(),
            images: None,
        },
        ChatMessage {
            role: "user".to_string(),
            content,
            images,
        },
    ]
}

/// Builds the classifier's single user turn: an optional conversation-context
/// block (last [`CLASSIFIER_HISTORY_TURNS`] turns as plain `Role: text` lines),
/// the latest message, today's date, and the trailing output instruction.
///
/// When `has_images` is true, appends [`CLASSIFIER_VISION_INSTRUCTION`] so the
/// model treats attached pixels as part of the routing decision.
fn build_classifier_user_turn(
    history: &[ChatMessage],
    latest_user_message: &str,
    today: &str,
    has_images: bool,
) -> String {
    let mut out = String::new();
    let context = recent_history_block(history);
    if !context.is_empty() {
        out.push_str(CONVERSATION_HEADER);
        out.push('\n');
        out.push_str(&context);
        out.push_str("\n\n");
    }
    out.push_str("Latest message: ");
    out.push_str(latest_user_message.trim());
    out.push_str("\n\nToday's date is ");
    out.push_str(today);
    out.push_str(".\n");
    out.push_str(CLASSIFIER_INSTRUCTION);
    if has_images {
        out.push('\n');
        out.push_str(CLASSIFIER_VISION_INSTRUCTION);
    }
    out
}

/// Formats the last [`CLASSIFIER_HISTORY_TURNS`] conversation turns as plain
/// `Role: text` lines for context. Returns an empty string when there is no
/// history. Assistant answers carry the entities that resolve an elliptical
/// follow-up ("what about X?"), so they are embedded too, but truncated to a
/// [`crate::config::defaults::CLASSIFIER_ASSISTANT_PREFIX_CHARS`] prefix to keep
/// the classifier prompt within its warm-slot budget; user turns (short
/// questions) are embedded whole. Prior-turn images are not re-attached here;
/// only the latest turn's images ride on the classifier user message when the
/// caller supplies them.
fn recent_history_block(history: &[ChatMessage]) -> String {
    let start = history
        .len()
        .saturating_sub(crate::config::defaults::CLASSIFIER_HISTORY_TURNS);
    history[start..]
        .iter()
        .map(|m| {
            if m.role == "assistant" {
                let answer = truncate_prefix(
                    m.content.trim(),
                    crate::config::defaults::CLASSIFIER_ASSISTANT_PREFIX_CHARS,
                );
                format!("Assistant: {answer}")
            } else {
                format!("User: {}", m.content.trim())
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Truncates `text` to at most `max_chars` characters on a character boundary,
/// appending a `…` marker when it had to cut so the classifier can tell the
/// answer was clipped. Counting by `char` (not byte) keeps a multi-byte prefix
/// from splitting a codepoint.
fn truncate_prefix(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let prefix: String = text.chars().take(max_chars).collect();
    format!("{prefix}…")
}

/// The wire shape the grammar constrains the model to. Parsed leniently: the
/// `search` string is normalised to [`SearchDecision`] here rather than via
/// serde so an unexpected casing does not hard-fail the whole response.
#[derive(serde::Deserialize)]
struct PrePassWire {
    search: String,
    #[serde(default)]
    route: String,
    #[serde(default)]
    standalone_question: String,
    #[serde(default)]
    queries: Vec<String>,
    /// `#[serde(default)]` so a model that omits the field (or any non-grammar
    /// caller) parses to `false` rather than hard-failing the whole response;
    /// the live grammar lists it in `required` so the model always emits it.
    #[serde(default)]
    explicit_search: bool,
    /// The language the model says the user wrote in. `#[serde(default)]` for
    /// the same lenient reason as `explicit_search`: a model that omits it
    /// degrades to an empty string (no language signal, so resolution simply
    /// falls through to the locale) rather than failing the whole response. The
    /// live grammar enum-constrains it AND lists it in `required`, so this is a
    /// degradation path, not the expected one.
    #[serde(default)]
    lang: String,
}

/// Parses a raw pre-pass response into a normalised decision, or `None` when
/// the body is not the expected JSON shape or the `search` value is unknown.
/// Queries are trimmed, de-duplicated case-insensitively, emptied entries
/// dropped, and capped at [`MAX_QUERIES`]; `standalone_question` is trimmed.
pub(crate) fn parse_prepass(raw: &str) -> Option<PrePassDecision> {
    let wire: PrePassWire = serde_json::from_str(raw.trim()).ok()?;
    let decision = match wire.search.trim().to_ascii_lowercase().as_str() {
        "no" => SearchDecision::No,
        "cached" => SearchDecision::Cached,
        "web" => SearchDecision::Web,
        _ => return None,
    };
    Some(PrePassDecision {
        decision,
        route: SearchRoute::from_wire(&wire.route),
        standalone_question: wire.standalone_question.trim().to_string(),
        queries: normalize_queries(wire.queries),
        explicit_search: wire.explicit_search,
        // Carried through verbatim, trimmed only. It is NOT validated here: the
        // one gate is `lang::resolve_lang`, so there is exactly one place an
        // unrecognised code can be turned away and no second, drifting copy of
        // the allowlist.
        lang: wire.lang.trim().to_string(),
    })
}

/// Resolves the final decision from a parse attempt, applying the failure
/// policy and backfilling required fields:
/// - a failed parse (`None`) becomes a `No` decision answering `latest`;
/// - a `Cached`/`Web` decision with no usable queries or an empty standalone
///   backfills from `latest` so the retrieval stages always have a query.
pub(crate) fn prepass_or_no(parsed: Option<PrePassDecision>, latest: &str) -> PrePassDecision {
    let mut decision = match parsed {
        Some(decision) => decision,
        None => {
            return PrePassDecision {
                decision: SearchDecision::No,
                route: SearchRoute::Web,
                standalone_question: latest.trim().to_string(),
                queries: Vec::new(),
                explicit_search: false,
                // No parse, so no language judgement: the resolver falls back to
                // the message's own script and the user's locale.
                lang: String::new(),
            };
        }
    };
    if decision.standalone_question.trim().is_empty() {
        decision.standalone_question = latest.trim().to_string();
    }
    if matches!(
        decision.decision,
        SearchDecision::Web | SearchDecision::Cached
    ) && decision.queries.is_empty()
    {
        decision.queries = vec![decision.standalone_question.clone()];
    }
    decision
}

/// Normalises a raw query list: trim, drop empties, de-duplicate
/// case-insensitively preserving first-seen order, cap at [`MAX_QUERIES`].
fn normalize_queries(raw: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for query in raw {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_ascii_lowercase()) {
            out.push(trimmed.to_string());
            if out.len() == MAX_QUERIES {
                break;
            }
        }
    }
    out
}

/// Scriptable [`PrePass`] for unit tests: returns a fixed decision or error so
/// the orchestrator's branch logic is driven without a live engine.
#[cfg(test)]
pub(crate) struct FakePrePass {
    result: Result<PrePassDecision, InferenceError>,
}

#[cfg(test)]
impl FakePrePass {
    pub(crate) fn returning(result: Result<PrePassDecision, InferenceError>) -> Self {
        Self { result }
    }
}

#[cfg(test)]
#[async_trait]
impl PrePass for FakePrePass {
    async fn decide(
        &self,
        _history: &[ChatMessage],
        _latest_user_message: &str,
        _latest_images: Option<&[String]>,
        _today: &str,
        _cancel: &CancellationToken,
    ) -> Result<PrePassDecision, InferenceError> {
        self.result.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(content: &str) -> ChatMessage {
        ChatMessage {
            role: "user".to_string(),
            content: content.to_string(),
            images: None,
        }
    }

    // ── schema ──────────────────────────────────────────────────────────────

    #[test]
    fn schema_declares_enum_and_query_bounds() {
        let s = prepass_schema();
        assert_eq!(s["properties"]["search"]["enum"][0], "no");
        assert_eq!(s["properties"]["route"]["enum"][0], "weather");
        assert_eq!(s["properties"]["route"]["enum"][3], "sports");
        assert_eq!(s["properties"]["route"]["enum"][4], "web");
        assert_eq!(s["properties"]["queries"]["maxItems"], MAX_QUERIES);
        assert_eq!(s["properties"]["explicit_search"]["type"], "boolean");
        assert!(s["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "route"));
        assert!(s["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "explicit_search"));
        assert_eq!(s["additionalProperties"], false);
    }

    #[test]
    fn schema_enum_constrains_lang_to_the_allowlist_and_requires_it() {
        let s = prepass_schema();
        let langs: Vec<String> = s["properties"]["lang"]["enum"]
            .as_array()
            .expect("lang is enum-constrained")
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        // The grammar's enum IS the allowlist, read from the same table the
        // outbound request shapes are read from, so the set the model may emit
        // and the set that can reach a hostname cannot drift apart.
        assert_eq!(langs, crate::websearch::lang::supported_langs());
        assert!(langs.contains(&"vi".to_string()));
        // Required: the field is the only signal that can name a language a
        // character rule cannot see, so a model free to omit it would omit it on
        // exactly the turns that need it.
        assert!(s["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "lang"));
    }

    // ── route parsing ─────────────────────────────────────────────────────────

    #[test]
    fn route_from_wire_maps_known_tiers() {
        assert_eq!(SearchRoute::from_wire("weather"), SearchRoute::Weather);
        assert_eq!(SearchRoute::from_wire("news"), SearchRoute::News);
        assert_eq!(SearchRoute::from_wire("wiki"), SearchRoute::Wiki);
        assert_eq!(SearchRoute::from_wire("sports"), SearchRoute::Sports);
        assert_eq!(SearchRoute::from_wire("web"), SearchRoute::Web);
        // Case-insensitive and whitespace-tolerant.
        assert_eq!(SearchRoute::from_wire("  NEWS "), SearchRoute::News);
    }

    #[test]
    fn route_from_wire_unknown_or_empty_falls_back_to_web() {
        assert_eq!(SearchRoute::from_wire("encyclopedia"), SearchRoute::Web);
        assert_eq!(SearchRoute::from_wire(""), SearchRoute::Web);
    }

    #[test]
    fn parse_reads_route_when_present() {
        let raw = r#"{"search":"web","route":"news","standalone_question":"q","queries":["a"]}"#;
        assert_eq!(parse_prepass(raw).unwrap().route, SearchRoute::News);
    }

    #[test]
    fn parse_defaults_route_to_web_when_missing_or_invalid() {
        // Missing route field entirely.
        let missing = r#"{"search":"web","standalone_question":"q","queries":["a"]}"#;
        assert_eq!(parse_prepass(missing).unwrap().route, SearchRoute::Web);
        // Present but not a known tier.
        let invalid =
            r#"{"search":"web","route":"maps","standalone_question":"q","queries":["a"]}"#;
        assert_eq!(parse_prepass(invalid).unwrap().route, SearchRoute::Web);
    }

    // ── message assembly ────────────────────────────────────────────────────

    #[test]
    fn messages_use_persona_free_classifier_system_prompt() {
        let msgs = build_prepass_messages(&[], "who won", None, "2026-07-05");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        // The classifier prompt, not the chat persona.
        assert_eq!(msgs[0].content, CLASSIFIER_SYSTEM);
        assert!(msgs[0].content.contains("retrieval-routing classifier"));
        assert!(msgs[0].content.contains("choose \"web\""));
        assert_eq!(msgs[1].role, "user");
        assert!(msgs[1].content.contains("Latest message: who won"));
        assert!(msgs[1].content.contains("2026-07-05"));
    }

    #[test]
    fn user_turn_embeds_recent_history_for_pronoun_resolution() {
        let history = vec![user("what is the capital of France"), {
            let mut m = user("Paris.");
            m.role = "assistant".into();
            m
        }];
        let msgs = build_prepass_messages(&history, "and its population?", None, "2026-07-05");
        let turn = &msgs[1].content;
        assert!(turn.contains("Conversation so far"));
        assert!(turn.contains("User: what is the capital of France"));
        assert!(turn.contains("Assistant: Paris."));
        assert!(turn.contains("Latest message: and its population?"));
    }

    #[test]
    fn user_turn_omits_conversation_block_when_no_history() {
        let msgs = build_prepass_messages(&[], "hello", None, "2026-07-05");
        assert!(!msgs[1].content.contains("Conversation so far"));
        assert!(msgs[1].content.starts_with("Latest message: hello"));
        assert!(msgs[1].images.is_none());
    }

    #[test]
    fn messages_attach_latest_images_and_vision_instruction() {
        let imgs = vec!["QUJD".to_string(), "REVG".to_string()];
        let msgs = build_prepass_messages(&[], "what is this?", Some(&imgs), "2026-07-05");
        assert_eq!(
            msgs[1].images.as_ref().map(|v| v.as_slice()),
            Some(imgs.as_slice())
        );
        assert!(msgs[1].content.contains(CLASSIFIER_VISION_INSTRUCTION));
        assert!(msgs[0].images.is_none());
    }

    #[test]
    fn empty_images_slice_stays_text_only() {
        let empty: [String; 0] = [];
        let msgs = build_prepass_messages(&[], "hello", Some(&empty), "2026-07-05");
        assert!(msgs[1].images.is_none());
        assert!(!msgs[1].content.contains(CLASSIFIER_VISION_INSTRUCTION));
    }

    #[test]
    fn history_block_keeps_only_the_most_recent_turns() {
        // More turns than the cap: only the last CLASSIFIER_HISTORY_TURNS survive.
        let cap = crate::config::defaults::CLASSIFIER_HISTORY_TURNS;
        let history: Vec<ChatMessage> = (0..cap + 3).map(|i| user(&format!("turn {i}"))).collect();
        let block = recent_history_block(&history);
        assert!(!block.contains("turn 0"));
        assert!(block.contains(&format!("turn {}", cap + 2)));
        assert_eq!(block.lines().count(), cap);
    }

    #[test]
    fn truncate_prefix_clips_long_text_and_keeps_short_text() {
        let bound = crate::config::defaults::CLASSIFIER_ASSISTANT_PREFIX_CHARS;
        // Short text is returned verbatim, no ellipsis.
        assert_eq!(truncate_prefix("brief", bound), "brief");
        // A string exactly at the bound is not clipped.
        let exact: String = "x".repeat(bound);
        assert_eq!(truncate_prefix(&exact, bound), exact);
        // One over the bound is clipped to `bound` chars plus the marker.
        let over: String = "y".repeat(bound + 50);
        let clipped = truncate_prefix(&over, bound);
        assert_eq!(clipped.chars().count(), bound + 1);
        assert!(clipped.ends_with('…'));
        assert!(clipped.starts_with(&"y".repeat(bound)));
    }

    #[test]
    fn truncate_prefix_cuts_on_a_char_boundary() {
        // Multi-byte codepoints must not be split mid-byte.
        let text: String = "é".repeat(10);
        let clipped = truncate_prefix(&text, 4);
        assert_eq!(clipped, "éééé…");
    }

    #[test]
    fn history_block_truncates_assistant_answers_but_not_user_turns() {
        let bound = crate::config::defaults::CLASSIFIER_ASSISTANT_PREFIX_CHARS;
        let long_answer = "a".repeat(bound + 100);
        let long_question = "b".repeat(bound + 100);
        let history = vec![
            {
                let mut m = user(&long_question);
                m.role = "user".into();
                m
            },
            {
                let mut m = user(&long_answer);
                m.role = "assistant".into();
                m
            },
        ];
        let block = recent_history_block(&history);
        // The assistant answer is clipped to the bound plus the marker.
        assert!(block.contains(&format!("Assistant: {}…", "a".repeat(bound))));
        assert!(!block.contains(&"a".repeat(bound + 1)));
        // The user question is embedded whole (short questions stay intact).
        assert!(block.contains(&format!("User: {long_question}")));
    }

    #[test]
    fn classifier_prompt_carries_ellipsis_resolution_fewshots() {
        // The topic-swap ellipsis example: same frame ("net worth"), swapped
        // subject, not an invented question type (the observed llama-3.2-3B bug).
        assert!(CLASSIFIER_SYSTEM.contains("How about Donald Trump?"));
        assert!(CLASSIFIER_SYSTEM.contains("what is Donald Trump's net worth"));
        // The pronoun-from-answer example resolves "he" from the prior answer.
        assert!(CLASSIFIER_SYSTEM.contains("how much is he worth?"));
        assert!(CLASSIFIER_SYSTEM.contains("what is Jensen Huang's net worth"));
        // The instruction to resolve from the assistant's answers, not only the
        // user's questions.
        assert!(CLASSIFIER_SYSTEM.contains("entities named in the assistant's previous answers"));
    }

    #[test]
    fn classifier_prompt_teaches_query_shape_diversity_for_quant_questions() {
        // Horizontal pin: ambiguous quantity questions must fan out distinct
        // answer shapes (rate vs level), put level first, not synonym restates.
        assert!(CLASSIFIER_SYSTEM.contains("rate versus a level or total"));
        assert!(CLASSIFIER_SYSTEM.contains("Vietnam nominal GDP USD"));
        assert!(CLASSIFIER_SYSTEM.contains("level-shaped query FIRST"));
        assert!(CLASSIFIER_SYSTEM
            .contains("\"Vietnam latest GDP\" -> {\"search\":\"web\",\"route\":\"web\""));
    }

    #[test]
    fn classifier_prompt_carries_the_language_preservation_rule() {
        // The rule: rewrite in the user's language, never translate to English.
        assert!(CLASSIFIER_SYSTEM.contains("SAME language the user wrote their latest message in"));
        assert!(CLASSIFIER_SYSTEM.contains("Never translate them into English"));
        // The English companion query is PERMITTED, not forbidden: the model
        // emits one on its own where it judges the English corpus useful, and
        // that hedge is worth keeping.
        assert!(CLASSIFIER_SYSTEM.contains("You may ADD one English query"));
        // Naming the language is a separate instruction from writing in it.
        assert!(CLASSIFIER_SYSTEM.contains("ISO 639-1 code of the language the USER wrote in"));
        // And it must not disturb the decision the rest of the prompt makes.
        assert!(CLASSIFIER_SYSTEM
            .contains("changes NOTHING about the \"search\" or \"route\" decision"));
        // The loanword trap, stated in the prompt as well as guarded in code.
        assert!(CLASSIFIER_SYSTEM.contains("what does pho mean\" is English"));
    }

    #[test]
    fn classifier_prompt_still_fits_the_output_budget() {
        // The `lang` field adds ~15 characters of OUTPUT. The cap governs output
        // tokens, not the prompt, and a classifier JSON object is far under it;
        // this asserts the headroom is real rather than assumed.
        assert!(crate::config::defaults::PREPASS_MAX_TOKENS >= 1536);
    }

    #[test]
    fn classifier_prompt_carries_search_trigger_taxonomy() {
        // Anthropic search-trigger category (c): entity/person/product
        // attributes that can change after the training cutoff.
        assert!(CLASSIFIER_SYSTEM.contains("can change after your training cutoff"));
        // The present-tense-attribute rule: a web turn even with no explicit
        // freshness word, demonstrated by an entity not yet searched this
        // conversation (the Pope's age), which cannot come from the cache.
        assert!(CLASSIFIER_SYSTEM.contains("even with no freshness word"));
        assert!(CLASSIFIER_SYSTEM.contains("how old is the Pope"));
        assert!(CLASSIFIER_SYSTEM.contains("how old is the current Pope"));
    }

    #[test]
    fn classifier_prompt_carries_explicit_search_and_drilldown_fewshots() {
        // Explicit look-it-up request: explicit_search true with a fully
        // resolved standalone question, never the literal "look it up".
        assert!(CLASSIFIER_SYSTEM.contains("can you look it up please?"));
        assert!(
            CLASSIFIER_SYSTEM.contains("what time is the Belgium vs Spain 2026 World Cup match")
        );
        assert!(CLASSIFIER_SYSTEM.contains("\"explicit_search\":true"));
        // A live or volatile drill-down (a kickoff time still ahead) stays web,
        // never cached, even when the topic was already searched.
        assert!(CLASSIFIER_SYSTEM.contains("A live or volatile detail is NEVER cached"));
        assert!(CLASSIFIER_SYSTEM.contains("at what exact time did it kick off?"));
        // A stable bio drill-down of an already-searched entity (an age) is
        // cached: the earlier profile sources plausibly carry it.
        assert!(CLASSIFIER_SYSTEM
            .contains("(you already searched Elon Musk's net worth this conversation"));
        assert!(CLASSIFIER_SYSTEM.contains("and how old is he now?"));
        assert!(CLASSIFIER_SYSTEM.contains("how old is Elon Musk"));
        // The (c)-clause carve-out: a present-tense attribute of an entity
        // ALREADY searched this conversation is cached, not web. Without this,
        // the "even with no freshness word -> web" rule wrongly re-searches a
        // biographical follow-up the reuse path can already ground.
        assert!(CLASSIFIER_SYSTEM.contains(
            "unless you already searched this person or entity earlier in this same conversation"
        ));
    }

    // ── parse ───────────────────────────────────────────────────────────────

    #[test]
    fn parse_reads_web_decision() {
        let raw = r#"{"search":"web","standalone_question":"weather in Paris today","queries":["paris weather today"]}"#;
        let d = parse_prepass(raw).unwrap();
        assert_eq!(d.decision, SearchDecision::Web);
        assert_eq!(d.standalone_question, "weather in Paris today");
        assert_eq!(d.queries, vec!["paris weather today"]);
    }

    #[test]
    fn parse_reads_lang_and_degrades_when_it_is_absent() {
        let raw = r#"{"search":"web","route":"web","standalone_question":"giá vàng hôm nay bao nhiêu","queries":["giá vàng hôm nay","gold price today"],"explicit_search":false,"lang":" vi "}"#;
        let d = parse_prepass(raw).unwrap();
        assert_eq!(d.lang, "vi");
        // The English companion query the model adds on its own survives intact:
        // it hedges toward the English corpus, which is useful, and it does NOT
        // change the turn's language (see `orchestrator::run_search`).
        assert_eq!(d.queries, vec!["giá vàng hôm nay", "gold price today"]);
        // A model that omits the field (no grammar) degrades to "no signal"
        // rather than failing the whole response.
        let missing = r#"{"search":"web","standalone_question":"q","queries":["a"]}"#;
        assert_eq!(parse_prepass(missing).unwrap().lang, "");
    }

    #[test]
    fn parse_reads_no_and_cached() {
        assert_eq!(
            parse_prepass(r#"{"search":"no","standalone_question":"hi","queries":["x"]}"#)
                .unwrap()
                .decision,
            SearchDecision::No
        );
        assert_eq!(
            parse_prepass(r#"{"search":"cached","standalone_question":"hi","queries":["x"]}"#)
                .unwrap()
                .decision,
            SearchDecision::Cached
        );
    }

    #[test]
    fn parse_is_case_insensitive_on_decision() {
        assert_eq!(
            parse_prepass(r#"{"search":"WEB","standalone_question":"q","queries":["a"]}"#)
                .unwrap()
                .decision,
            SearchDecision::Web
        );
    }

    #[test]
    fn parse_rejects_unknown_decision() {
        assert!(
            parse_prepass(r#"{"search":"maybe","standalone_question":"q","queries":["a"]}"#)
                .is_none()
        );
    }

    #[test]
    fn parse_rejects_non_json() {
        assert!(parse_prepass("not json at all").is_none());
    }

    #[test]
    fn parse_normalizes_queries() {
        let raw =
            r#"{"search":"web","standalone_question":"q","queries":["  A ","a","B","","C","D"]}"#;
        let d = parse_prepass(raw).unwrap();
        // trimmed, case-insensitive dedupe ("A"=="a"), empty dropped, capped at 3
        assert_eq!(d.queries, vec!["A", "B", "C"]);
    }

    #[test]
    fn parse_reads_explicit_search_flag() {
        let raw =
            r#"{"search":"web","standalone_question":"q","queries":["a"],"explicit_search":true}"#;
        assert!(parse_prepass(raw).unwrap().explicit_search);
    }

    #[test]
    fn parse_defaults_explicit_search_false_when_missing() {
        // The field is absent: serde default keeps the response parseable and
        // treats it as a non-explicit turn.
        let raw = r#"{"search":"web","standalone_question":"q","queries":["a"]}"#;
        assert!(!parse_prepass(raw).unwrap().explicit_search);
    }

    // ── failure policy / backfill ───────────────────────────────────────────

    #[test]
    fn none_becomes_no_answering_latest() {
        let d = prepass_or_no(None, "what is 2 + 2");
        assert_eq!(d.decision, SearchDecision::No);
        assert_eq!(d.standalone_question, "what is 2 + 2");
        assert!(d.queries.is_empty());
    }

    #[test]
    fn web_with_empty_queries_backfills_from_standalone() {
        let parsed = Some(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: "capital of France".into(),
            queries: vec![],
            explicit_search: false,
            lang: "en".into(),
        });
        let d = prepass_or_no(parsed, "and there?");
        assert_eq!(d.decision, SearchDecision::Web);
        assert_eq!(d.queries, vec!["capital of France"]);
    }

    #[test]
    fn web_with_empty_standalone_backfills_from_latest() {
        let parsed = Some(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: "   ".into(),
            queries: vec!["q".into()],
            explicit_search: false,
            lang: "en".into(),
        });
        let d = prepass_or_no(parsed, "the real question");
        assert_eq!(d.standalone_question, "the real question");
        assert_eq!(d.queries, vec!["q"]);
    }

    #[test]
    fn no_decision_passes_through_without_query_backfill() {
        let parsed = Some(PrePassDecision {
            decision: SearchDecision::No,
            route: SearchRoute::Web,
            standalone_question: "hello".into(),
            queries: vec![],
            explicit_search: false,
            lang: "en".into(),
        });
        let d = prepass_or_no(parsed, "hello");
        assert_eq!(d.decision, SearchDecision::No);
        assert!(d.queries.is_empty());
    }

    // ── trait / fake ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn fake_prepass_returns_scripted_decision() {
        let want = PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: "q".into(),
            queries: vec!["q".into()],
            explicit_search: false,
            lang: "en".into(),
        };
        let fake = FakePrePass::returning(Ok(want.clone()));
        let got = fake
            .decide(&[], "q", None, "2026-07-05", &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(got, want);
    }

    #[tokio::test]
    async fn fake_prepass_propagates_error() {
        let fake = FakePrePass::returning(Err(InferenceError::Cancelled));
        assert_eq!(
            fake.decide(&[], "q", None, "2026-07-05", &CancellationToken::new())
                .await,
            Err(InferenceError::Cancelled)
        );
    }
}
