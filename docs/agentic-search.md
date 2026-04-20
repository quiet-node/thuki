# Agentic Search in Thuki

A deep technical tour of the `/search` command: what it is, how every stage works, why we built it this way, and how it stays entirely on the user's machine.

---

## 1. What this is, in one paragraph

Thuki's `/search` command is an agentic retrieval pipeline. The user types a question; a local LLM decides whether a web search is needed and what to search for; a privacy-respecting meta-search engine returns candidate URLs; a reranker picks the strongest matches; a lightweight judge asks "do these snippets actually contain the answer?"; if the snippets fall short, a reader service fetches the full page bodies; the judge looks again at the richer material; if the answer still is not fully there, the loop generates follow-up queries and goes around again, up to a bounded number of iterations; finally the answer is synthesized from the accumulated evidence and streamed back to the user with inline citations. Every piece of this runs on the user's machine. No cloud calls, no API keys, no telemetry.

If you have used Perplexity or ChatGPT Search, this is the same shape of system, rebuilt around local-first infrastructure.

---

## 2. Why a local-first agentic pipeline at all

### The problem with a plain local LLM

A small local LLM (Thuki's default is Gemma 4 e2b, a 2B parameter model) has two structural limits. First, it only knows what was in its training corpus, and that corpus has a cutoff. The model genuinely does not know that Elon Musk now owns Twitter, or that curl shipped a fix for a CVE last week, or that the release notes for your library say what they say. Second, small models hallucinate more than large ones when they are asked about specifics they are not sure of.

Agentic search fixes both. Instead of asking the model to recall, we ask it to reason over real, fresh, cited sources that we retrieved seconds ago. The model's job shrinks from "know everything" to "synthesize the evidence in front of you." That is a much easier job, and it is exactly the job small models can do well.

### The problem with a plain RAG pipeline

A naive retrieval-augmented generation pipeline looks like this: user query goes to a search API, top results come back, all their text is stuffed into a prompt, the model writes an answer. This works for easy questions. It fails on harder ones for three reasons:

1. **The top search results may not contain the answer.** Search engines rank by relevance to the query, not by completeness of the answer. A question like "who are the Hedera Council members and can you list them all?" is easy to match on the word "Hedera" but the actual answer requires a page that enumerates all members, which may not be the top hit.

2. **Snippets are too short.** Most meta-search engines return ~150 character snippets per result. That is enough to verify the page is relevant but almost never enough to produce a substantive answer.

3. **A single retrieval is not enough for compound questions.** "Who founded Twitter? Who owns it now?" is two distinct factual lookups. One search query rarely surfaces sources that cover both well.

Agentic search solves these by adding an explicit decision layer. After retrieving, we stop and ask: "Do we actually have what we need to answer? If not, what is missing and what should we search for next?" Then we act on the answer.

### The problem with cloud-hosted agentic search

Products like Perplexity, ChatGPT Search, and You.com ARI solve all of the above brilliantly, but they ship every query to a vendor's servers. For people whose queries are about personal projects, unreleased code, legal questions, medical questions, or anything else they would rather not hand to a third party, that is a hard blocker.

Thuki's `/search` runs the entire loop locally. SearXNG (the meta-search engine) runs in a hardened Docker container on the user's machine. The reader service runs in another hardened container. The LLM runs in a third. The Rust backend orchestrates them. Queries never leave the user's machine except to hit the upstream search engines that SearXNG aggregates, and those are already public web services the user could hit directly.

---

## 3. The shape of the pipeline

Here is the complete control flow for a `/search` invocation. Each box is implemented in Rust and each arrow is a single function call or an event emitted to the frontend.

```
                 ┌────────────────────────────────────┐
                 │  User types /search <question>     │
                 └──────────────────┬─────────────────┘
                                    │
                                    ▼
                 ┌────────────────────────────────────┐
                 │  Merged router + judge LLM call    │
                 │  (single round-trip to Ollama)     │
                 └──┬────────────┬───────────────┬────┘
                    │            │               │
            CLARIFY │   PROCEED+sufficient      PROCEED+partial
                    │   (answer lives in        PROCEED+insufficient
                    │    conversation history)         │
                    ▼            ▼               │
           ┌────────────┐  ┌───────────┐         │
           │ Stream     │  │ Stream    │         │
           │ follow-up  │  │ answer    │         │
           │ question   │  │ from      │         │
           │ token      │  │ history   │         │
           │ by token   │  │ context   │         │
           └─────┬──────┘  └─────┬─────┘         │
                 │               │               │
              done            done               ▼
                                     ┌─────────────────────┐
                                     │ SearXNG search      │
                                     │ (parallel, dedup)   │
                                     └─────────┬───────────┘
                                               │
                                               ▼
                                     ┌─────────────────────┐
                                     │ BM25F + RRF rerank  │
                                     │ → TOP_K_URLS (10)   │
                                     └─────────┬───────────┘
                                               │
                                               ▼
                                     ┌─────────────────────┐
                                     │ Snippets judge      │
                                     │ (pre-synth verdict) │
                                     └──┬───────────┬──────┘
                                        │           │
                                 sufficient      partial /
                                        │       insufficient
                                        ▼           │
                                     synth          ▼
                                               ┌─────────────────────┐
                                               │ Reader fetches top  │
                                               │ URLs in parallel    │
                                               └─────────┬───────────┘
                                                         │
                                                         ▼
                                               ┌─────────────────────┐
                                               │ Chunker splits each │
                                               │ page into ~500-token│
                                               │ chunks with URL tag │
                                               └─────────┬───────────┘
                                                         │
                                                         ▼
                                               ┌─────────────────────┐
                                               │ BM25 chunk rerank   │
                                               │ → TOP_K_CHUNKS (8)  │
                                               └─────────┬───────────┘
                                                         │
                                                         ▼
                                               ┌─────────────────────┐
                                               │ Chunks judge        │
                                               └──┬───────────┬──────┘
                                                  │           │
                                           sufficient      partial /
                                                  │       insufficient
                                                  ▼           │
                                               synth          ▼
                                                     ┌───────────────────┐
                                                     │ Gap loop (k ≤ 3): │
                                                     │ judge emits new   │
                                                     │ queries; loop     │
                                                     │ back to SearXNG   │
                                                     └────────┬──────────┘
                                                              │
                                                              ▼
                                                   loop until sufficient
                                                   or iteration cap hit
                                                              │
                                                              ▼
                                                   ┌────────────────────┐
                                                   │ Exhaustion         │
                                                   │ fallback: synth    │
                                                   │ from best chunks + │
                                                   │ warning            │
                                                   └────────────────────┘
```

The rest of this document walks each box in detail.

---

## 4. Stage 1: The merged router + judge call

### What it is

Before we do anything expensive, we ask the local LLM a single question: what should we do with this user input? The call returns a JSON object:

```json
{
  "action": "clarify" | "proceed",
  "clarifying_question": string | null,
  "history_sufficiency": "sufficient" | "partial" | "insufficient" | null,
  "optimized_query": string | null
}
```

The router call and the "can we answer from history?" judge are merged into a single LLM round-trip. One prompt, four-field output, one inference pass. This was a conscious design choice: two back-to-back calls would cost us an extra round-trip on every query.

### Three branches from one call

**`action == "clarify"`.** The user's query is ambiguous and cannot be interpreted without more information. The pipeline streams the clarifying question to the frontend as ordinary `Token` events and stops. No search happens. Common triggers are pronouns with no antecedent ("who is he?" in a new conversation), missing subjects, or scope that could be interpreted several incompatible ways.

**`action == "proceed"` with `history_sufficiency == "sufficient"`.** The answer is literally already in the conversation transcript (a prior turn contained the exact fact). The pipeline skips the web search entirely and streams a synthesis from the conversation history using a dedicated "answer from context" prompt. This is the cheap path. Important: training knowledge does NOT qualify as "sufficient." The user invoked `/search` because they want fresh web data; the router only short-circuits when the transcript literally contains the answer.

**`action == "proceed"` with `history_sufficiency != "sufficient"`.** This is the default path for `/search`. The optimized query is sent to SearXNG and the retrieval pipeline runs end to end.

### Resilience: retry, then fall back

Small local models occasionally produce malformed JSON. If the first attempt does not parse, the router retries once with a stricter suffix appended to the user message ("Respond with ONLY the JSON object described by the system prompt. No prose, no markdown fences, no explanation."). If the retry also fails, the pipeline does not error. It falls back to a safe default: `action=proceed`, `history_sufficiency=insufficient`, `optimized_query=<raw user query>`. This means `/search` always produces an answer even when the router temporarily glitches. A cryptic "Search analysis failed" error bubble is never shown.

### The prompt

The router prompt is at `src-tauri/prompts/search_plan.txt`. It is deliberately terse (models follow short instructions better than long ones) and it injects today's UTC date via a `{{TODAY}}` placeholder so the model anchors any time-sensitive routing to the real calendar date, not its training cutoff.

### Why "merged" and not "chained"

Some designs call the router first, get `action=proceed`, then call a separate "history-sufficiency judge" to decide whether to search. That is two LLM calls before any useful work begins. By merging, we pay one call. The tradeoff is that the router prompt has to carry a bit more schema, which is a mild cost. On a 2B-parameter local model, one call vs two is the difference between the user feeling responsiveness and the user noticing lag.

---

## 5. Stage 2: SearXNG search

### What it is

SearXNG is a self-hosted, open-source meta-search engine. It aggregates results from 70+ upstream engines (Google, Bing, DuckDuckGo, Brave, Wikipedia, Stack Overflow, GitHub, arXiv, and many more) and returns a normalized JSON response. Our instance is locked to `127.0.0.1:25017`; there is no way to reach it from outside the user's machine.

We chose SearXNG over alternatives because:

- **Paid Bing/Google APIs** require an account, a credit card, and send every query to the vendor. They also have tight rate limits on their free tiers.
- **Scraping public search engines directly** is fragile; engines block the pattern within days.
- **SearXNG** is a single drop-in container that normalizes many engines into one API, runs entirely locally, and has no vendor relationship.

### How we call it

The Rust side hits `GET /search?q=<query>&format=json` over HTTP. Response bodies are parsed into a `SearxResult` struct (title, URL, content snippet, score, engine). HTML entities are decoded (`&amp;` → `&`, `&lt;` → `<`, numeric entities, etc.) and long fields are truncated character-by-character to avoid splitting UTF-8 codepoints.

On a gap-analysis round, we fan out multiple queries in parallel via `search_all_with_endpoint`. Per-query failures are silently dropped so one flaky engine cannot poison the batch. The union of results is deduplicated by URL before reranking.

### Security posture

Because SearXNG reaches out to the open internet, we harden the container aggressively: `cap_drop: ALL` except for the minimum capabilities uwsgi needs (`CHOWN`, `SETGID`, `SETUID`), `no-new-privileges: true`, localhost-only port binding, and no shared bind mounts with the host. The SearXNG settings file is mounted read-write inside the container but lives in the source tree so users can edit engine preferences without rebuilding.

---

## 6. Stage 3: BM25F + RRF URL reranking

### Why a second ranking stage?

SearXNG returns results already ranked by its upstream engines. For most queries, that ranking is fine. For Thuki's purposes, though, we want to feed the best ~5 URLs to the judge and the reader, and the engine's own ranking is tuned for "most clicked" rather than "most relevant to our specific query." Adding a small reranker closes that gap at negligible cost (runs in microseconds on the CPU).

### What BM25F does

BM25F is a generalization of the venerable Okapi BM25 retrieval function that allows weighting different fields of a document differently. We treat each SearXNG result as a document with two fields:

- **Title** (weight 2.0). A matching title is a strong signal.
- **Content snippet** (weight 1.0).

We compute a BM25F score against the query for each result.

### What RRF fuses with it

Reciprocal Rank Fusion is a classical technique for combining two rankings into one. It is used by Elasticsearch and OpenSearch as the default hybrid ranker. The formula:

```
RRF_score(d) = sum over ranked lists L of  1 / (k + rank_L(d))
```

with `k = 60` (the canonical Elasticsearch default). We fuse:

1. **Our BM25F ranking** (query-relevance signal).
2. **SearXNG's original engine ordering** (popularity / authority signal).

The fused score balances the two. A result that ranks well by both signals rises to the top. A result that ranks high on only one still has a chance, but does not dominate.

### Why single-field BM25 for chunks

When we rerank the chunks produced by the reader (later in the pipeline), we use vanilla single-field BM25 rather than BM25F + RRF. The reason: chunks have only one field (the text body) and there is no secondary engine-order signal to fuse with. BM25 is the right tool at that scale.

---

## 7. Stage 4: The sufficiency judge

### What it is and why it exists

After SearXNG returns results and we rerank them, we could just stuff the top snippets into a synthesis prompt and call it a day. That is what a naive RAG pipeline does. The problem is that snippets are often not enough, and we have no way to know from the outside. The judge is a small dedicated LLM call whose only job is to answer one question: "do these sources contain what we need to produce a substantive answer?"

The judge emits a structured verdict:

```json
{
  "sufficiency": "sufficient" | "partial" | "insufficient",
  "reasoning": "one short sentence for debug logs",
  "gap_queries": ["what's missing", "another angle", "third query"]
}
```

### The three verdicts and what they mean in practice

- **`sufficient`**: the sources contain both the literal answer AND the supporting facts that make the answer substantive. Dates, numbers, named entities, context. A reader should be able to produce a good two-paragraph answer from the material. This is the bar for shortcutting through to synthesis.

- **`partial`**: the sources address the question but leave clear gaps. The literal answer may be there, but dates are missing, or numbers are missing, or a comparison only covers some of the alternatives. This is the expected verdict for most entity / event / comparison / how-to questions when the judge is looking at SearXNG snippets, because snippets rarely carry enough context. Partial triggers the reader to fetch full pages.

- **`insufficient`**: the sources do not actually answer the question. Triggers both a reader fetch (for whatever material we do have) and, on the next iteration, a gap-query round with fresh searches.

When the verdict is anything other than `sufficient`, the judge also populates `gap_queries` with up to three distinct follow-up search queries that target the missing facts. These are not trivial rewordings of the original question; they are new angles. A good judge run on "compare tokio vs async-std performance" might return `["tokio vs async-std benchmark 2026", "async-std maintenance status 2026", "tokio async-std memory comparison"]`.

### Parse tolerance

The judge response is parsed with a custom tolerant JSON extractor in `src-tauri/src/search/judge.rs`. It finds the first balanced `{...}` pair in the raw model output, stripping any chatty preamble or markdown fences the model sometimes produces against instructions. After parsing, a normalization pass enforces invariants:

- Empty / whitespace-only `gap_queries` are dropped.
- The list is truncated to `GAP_QUERIES_PER_ROUND` (default 3).
- When `sufficiency == "sufficient"`, `gap_queries` is cleared even if the model returned some (the invariant is that a sufficient verdict never has outstanding gaps).

These are code-enforced guarantees. Downstream pipeline code can rely on them without re-validating.

---

## 8. Stage 5: The reader service

### What it is

When the judge says "partial" or "insufficient" on snippets, the pipeline escalates to the reader. The reader is a small FastAPI service (`sandbox/search-box/reader/main.py`, about 90 lines) running in its own hardened Docker container. It has one endpoint, `POST /extract`, which takes a URL and returns LLM-ready markdown of the page's main content.

```
Request:   { "url": "https://example.com/article" }
Response:  { "url": "...",
             "title": "Page title",
             "markdown": "# Article text\n\nClean body...",
             "status": "ok" | "empty" }
```

### Why Trafilatura

HTML boilerplate removal is a surprisingly hard problem. Navigation menus, ads, cookie banners, footers, related-article widgets, and content embeds all contaminate the signal. Naive approaches (strip `<nav>`, `<footer>`) fail on modern SPAs where everything is `<div>`. Doing it well requires heuristics built over years of NLP research.

We evaluated (and rejected):

- **Firecrawl**: AGPL-3.0 license, which is a blocker for a shipped desktop product.
- **Jina Reader cloud**: proxies every URL through Jina's servers, violating the privacy-first constraint.
- **Crawl4AI**: requires a 4 GB RAM Chromium browser in-container and has a CVE history.
- **ScrapeGraphAI and ReaderLM-v2**: use an LLM call per page extraction, which doubles the inference cost.
- **DIY headless browser**: SSRF surface without the boilerplate-removal value Trafilatura provides.
- **Rust-native readability crates**: the January 2025 benchmark showed most of them return empty strings on real pages.

**Trafilatura** wins on every axis we care about. F1 score around 0.95 on the ScrapingHub article extraction benchmark (top of the field). Apache 2.0 license. Production use at HuggingFace, IBM, Microsoft Research, Stanford, and the EU Parliament. Pure Python, no browser, tiny attack surface. 71 MB final Docker image.

### Security posture

The reader enforces defense in depth at three layers:

**Application layer (`main.py`)**
- SSRF guard: rejects non-http(s) schemes, private IPv4 ranges (RFC1918, loopback, link-local, multicast, reserved), IPv6 private/loopback/link-local ranges, and the literal string `"localhost"`. Both IP-literal and named hosts are checked.
- Byte cap: fetch aborts once 2 MB is buffered so a hostile server cannot exhaust memory by streaming an infinite response.
- Timeout: 8-second hard ceiling on upstream fetch.
- Request body limits: URL max length 2048 characters, validated by Pydantic.

**Container layer (`docker-compose.yml`)**
- `cap_drop: ALL` (no Linux capabilities at all, not even the reduced set SearXNG retains).
- `no-new-privileges: true`.
- `read_only: true` root filesystem.
- `tmpfs: /tmp:size=16m` for the minimal writable scratch area.
- `mem_limit: 512m`, `cpus: 1.0`.
- Bound to `127.0.0.1:25018` only.

**Image layer (`Dockerfile`)**
- Runs as `reader:reader` (UID/GID 10001, system user, no home directory).
- Only `main.py`, pinned runtime deps, and ca-certificates land in the image. No pytest, no dev tools, no compilers.

### Batch fetching on the Rust side

The Rust client (`src-tauri/src/search/reader.rs`) orchestrates a batch fetch. It uses a 5-slot semaphore so we never have more than five in-flight HTTP requests to the reader at once; each per-URL request carries a 10-second timeout; the whole batch carries a 30-second timeout. A cancellation token threads through every `await` point, so if the user dismisses the overlay mid-fetch the entire batch drops immediately.

The client classifies outcomes into five buckets:

- `Page`: real markdown content, goes downstream.
- `Empty`: reader returned `status == "empty"`, meaning Trafilatura extracted nothing meaningful. Probably a JS-rendered page. Recorded in pipeline metadata so we can later decide whether a Playwright v2 is worth adding.
- `Failed`: non-2xx HTTP response, or JSON decode failure. The URL is dropped silently.
- `ServiceUnavailable`: transient connect error (for example the reader container is down). If EVERY URL in the batch returns this, the function returns `ReaderError::ServiceUnavailable` so the pipeline can emit a warning and fall back to snippets.
- `Cancelled`: propagates `ReaderError::Cancelled` up.

### The single-retry rule

A single transient failure gets one retry with a short backoff (500 ms). Semantic failures (HTTP 404, malformed JSON, page extracts to empty) never retry; they are not going to change on the second attempt. This rule is shared across the LLM, SearXNG, and reader callers through a tiny `errors::retry_once` helper.

---

## 9. Stage 6: Chunking and chunk reranking

### Why chunk at all

Reader pages can be long. A Wikipedia article about Tesla might be 40,000 characters. Putting that straight into the judge's prompt would blow the context budget of a small local model several times over. We need to pick the most relevant segments.

Chunking splits each page into roughly 500-token pieces (words, approximated as whitespace-separated tokens). Paragraph boundaries (blank-line separated blocks) are preserved when possible. Oversized single paragraphs fall back to word-group slicing. Each chunk carries the URL and title of its source page so citations can later map back correctly.

### Why chunk rerank uses vanilla BM25

The chunk reranker treats each chunk as a single-field document (the text body) and scores it against the original user query. No BM25F (there are no fields to weight). No RRF (there is no secondary ranking signal to fuse with). Stable ordering for ties via the original-index tiebreaker so the output is deterministic.

We take the top `TOP_K_CHUNKS` (default 8) across all accumulated chunks from all rounds. This is the material the judge and the synthesizer actually see.

### The URL dedup invariant

Before the chunks are handed to the judge, we deduplicate by source URL, keeping only the highest-ranked chunk per URL. This enforces a useful invariant: the citation index `[k]` in the synthesis output always maps to a distinct source URL. Without the dedup, if the reranker picked eight chunks that all came from two Wikipedia pages, the model might emit `[1]`, `[2]`, ..., `[8]` while the Sources footer would show only two unique URLs: classic citation/source mismatch. The dedup makes the two match exactly.

---

## 10. Stage 7: The gap-analysis loop

### The intuition

After the first pass (SearXNG + rerank + reader + chunk rerank + chunks judge), one of three things is true:

1. **The judge says sufficient.** Great, we synthesize.
2. **The judge says partial or insufficient and has useful gap queries.** We run the gap queries through SearXNG, add the new URLs to the pool, fetch them with the reader, add their chunks, rerank globally, and judge again.
3. **The judge emits no gap queries even though it said not-sufficient.** The loop breaks; we fall back to synthesizing from whatever we have.

This is the classic CRAG / Self-RAG / FLARE pattern adapted to our constraints. Each iteration has the same shape as the initial round, and each iteration can produce a "sufficient" verdict that ends the loop early.

### Why a hard iteration cap

Unbounded loops are a known footgun in agentic systems. A confused judge can keep saying "insufficient" forever, burning inference cycles and user patience. We cap at `MAX_ITERATIONS = 3` (initial pass + 2 gap rounds) as a hard guarantee of termination. Empirically this is enough to handle multi-hop questions ("what is the population of Vietnam divided by the area of Texas?") while staying under the ~45-60 second budget that makes `/search` feel interactive.

### What the user sees during the loop

The frontend gets a stream of events. Each gap round emits:

- `RefiningSearch { attempt: k, total: N }` (drives the `Refining search (2/3)` label).
- `Searching` (on the gap-round SearXNG fanout).
- `Sources { results }` (gap-round URLs, reranked).
- `ReadingSources` (gap-round reader fetches).
- Possibly `Warning { warning: ... }` if the reader has a problem.
- Eventually `Composing` and then a stream of `Token` events.

Once `RefiningSearch` has fired for this turn, the frontend flips an `inGapRound` flag that changes the copy on subsequent stage labels: `Searching the web` becomes `Searching more angles`; `Reading sources` becomes `Reading additional pages`; `Composing answer` becomes `Composing refined answer`. The user sees that Thuki is doing deeper work, not repeating the first round.

### Exhaustion fallback

If we run through all `MAX_ITERATIONS` without the judge reporting `sufficient`, we do NOT error. We synthesize from the best accumulated chunks we have, attach an `IterationCapExhausted` warning to the turn, and let the user see a caveated answer ("Answer based on limited information. Try a more specific question for better results.") rather than a failure bubble. A weaker answer with a caveat is better UX than an error.

---

## 11. Stage 8: Synthesis and streaming

### Assembling the prompt

At synthesis time we build a multi-turn chat message list:

1. **System prompt**: the synthesis system prompt from `src-tauri/prompts/search_synthesis.txt`, with `{{TODAY}}` replaced by today's UTC date. This contains the substance rules, citation format, and the concrete few-shot example that shows what a good answer looks like for a "Who founded X?" question.
2. **Prior conversation history**: all completed user and assistant turns that existed before this `/search` invocation. Gives the model conversational continuity.
3. **The user turn**: the user's raw query.
4. **A synthetic system message containing the numbered sources**: built from the final `judge_sources` slice, which is already deduplicated by URL. Each source includes title, URL, and text.

The model is explicitly told that the sources are private reference material, not a document it is describing to the user.

### Why we emphasize substance

Small local models default to one-sentence answers when they are told to "answer concisely." The synthesis prompt aggressively pushes the other direction:

- Open with the direct literal answer.
- Then add the supporting context a curious reader would naturally want: for people, their role and notable facts; for companies, founding year and what they do; for events, when, where, and why they matter; for processes, the essential steps and outcomes; for comparisons, the axes that matter.
- Anticipate the obvious follow-up.
- Aim for substance, not length. Two to four tight paragraphs is typical. Never pad.

The prompt also contains a concrete "Who founded Tesla?" few-shot example with an unacceptable one-liner next to an acceptable multi-sentence grounded answer. Showing the pattern works far better than describing it on small models.

### Citation format

Inline markers `[1]`, `[2]`, `[1][3]` for claims supported by multiple sources. The number `[k]` references position `k` in the numbered sources list the model was given. Because the sources were deduplicated by URL upstream, each `[k]` maps to a distinct URL in the Sources footer.

### Streaming and cancellation

Synthesis uses `stream_ollama_chat`, Ollama's newline-delimited JSON streaming protocol. Each chunk arrives as a `Token` event and the frontend appends it to the assistant bubble in real time. The cancellation token is woven into the streaming loop: if the user dismisses the overlay mid-stream, we drop the HTTP connection (which signals Ollama to stop inference), emit a `Cancelled` event, and do not persist the partial turn.

### The final Sources event

Immediately before emitting `Composing`, we emit one authoritative `Sources` event with the exact URL list the synthesis prompt received (post-dedup). The frontend replaces `pendingSources` with this final list. This guarantees that by the time the user sees `[4]` or `[5]` in the answer, the Sources footer contains at least that many entries in the same order. No more "citation references position 5 but footer only has 2 items."

---

## 12. Persisted metadata and the iteration trace

Every `/search` turn writes two extra columns to SQLite beyond the normal turn body: `search_warnings` and `search_metadata`. Both are nullable JSON-serialized TEXT fields. They never surface to the UI. They exist for three reasons:

1. **Debugging user reports.** When a user says "this answer was bad," the persisted trace shows exactly which queries were issued, which URLs were fetched, how long each stage took, what the judge's reasoning was at each step, and which iteration produced the final answer.

2. **Empty-body URL telemetry.** Each iteration trace records `reader_empty_urls`: URLs where Trafilatura succeeded in fetching but extracted nothing (typical for JS-rendered pages). If this list grows consistently for a subset of popular domains, it is a concrete data signal that we should invest in a Playwright-style renderer as a v2 addition.

3. **Regression testing.** When we change the prompt or the reranker, we can replay historical traces and see whether judge verdicts shift in meaningful ways.

The `IterationTrace` struct records, for each round:

- `stage`: `Initial` or `GapRound { round }`.
- `queries`: the SearXNG queries issued this round.
- `urls_fetched`: URLs passed to the reader.
- `reader_empty_urls`: URLs where the reader returned empty.
- `judge_verdict`: `sufficient` / `partial` / `insufficient`.
- `judge_reasoning`: the one-sentence explanation.
- `duration_ms`: how long the round took.

---

## 13. Graceful degradation and warnings

Things can break in a lot of places. The pipeline tolerates all of them rather than erroring out when there is a reasonable fallback.

| Failure mode | Detection | User-facing outcome |
|---|---|---|
| Reader container down | All reader URLs return connect-refused | `ReaderUnavailable` warning, synthesis falls back to SearXNG snippets, pipeline continues |
| Reader batch times out | 30-second wall-clock ceiling exceeded | `ReaderPartialFailure` warning, synthesis uses whatever partial results arrived in time |
| More than 50% of reader URLs fail | Counted post-batch | `ReaderPartialFailure` warning, synthesis proceeds with what succeeded |
| Router returns malformed JSON twice | Two parse failures in a row | Silent fallback to `proceed + insufficient + raw query`, pipeline runs a fresh search |
| SearXNG returns zero initial results | Empty response body | `NoResultsInitial` warning, error bubble "No search results found. Try rephrasing." |
| Gap round returns zero results | Empty response body | Silent: the round counter advances, loop tries the next gap queries or exhausts |
| Gap loop exhausts without sufficient | All `MAX_ITERATIONS` done, still not sufficient | `IterationCapExhausted` warning, synthesis from best accumulated chunks with a caveat |
| Synthesis stream interrupted mid-token | Stream ends before `Done` | `SynthesisInterrupted` warning, what was streamed so far is persisted |
| Cancellation token fires at any await | `tokio::select!` fires the cancel branch | `Cancelled` event, partial state dropped, nothing persisted |

Warnings accumulate across the turn and are shown as a small amber or red icon beside the `Sources (k)` collapsible in the chat bubble. Hover reveals the plain-English message through the shared Tooltip component. The icon is the signal that something did not go perfectly; the tooltip carries the detail. We do not interrupt the user mid-conversation with modals for any of these conditions.

---

## 14. The sandbox architecture

Thuki ships three local services in two separate Docker compose stacks.

### `sandbox/llm-box/`

Runs Ollama. The compose file is built on a shared `x-ollama-base` anchor that enforces `cap_drop: ALL` and `no-new-privileges: true`. An init service pulls the model into a read-write volume, then exits. A long-running service mounts the same volume read-only and serves inference on `127.0.0.1:11434`. Default model is Gemma 4 e2b (a 2B-parameter multilingual model), overridable via the `OLLAMA_MODEL` env var. No external network ingress; localhost-only binding is the only way in.

### `sandbox/search-box/`

Runs SearXNG and the reader, together, on a shared bridge network called `search_net`. Both services are pinned to `127.0.0.1` on distinct ports (25017 for SearXNG, 25018 for reader). Each service is hardened independently (see sections 5 and 8). They can talk to each other through `search_net` if we ever want reader-to-searxng wiring, but currently every request comes from the Rust backend on the host.

### Why two separate compose stacks

Users who only want chat (no `/search`) can bring up just `llm-box` and use Thuki normally. The `/search` command becomes unavailable but the app still works. This separation mirrors how the functionality is actually layered: LLM is required; search is optional. The two stacks have distinct lifecycles and distinct on-disk state (models in one volume, SearXNG settings in a committed directory).

---

## 15. How this compares to the industry

The 2025-2026 agentic-search landscape has converged on a few reference architectures. Here is how Thuki sits relative to them.

| Product | Router | Judge | Gap loop | Reader | Unified gate | Runs locally |
|---|---|---|---|---|---|---|
| Perplexity Pro / Sonar | depth-based | implicit via rerank | yes | yes | distributed | no |
| Perplexity Deep Research | planner | yes | yes | yes | single loop | no |
| ChatGPT Search (GPT-5) | depth-based | partial | yes | yes | no | no |
| OpenAI Deep Research (o3) | triage agent | yes | yes | yes | single research agent | no |
| Exa Deep Research | planner + observer | yes | yes | yes | observer-centered | no |
| You.com ARI | planner | yes | yes | yes | partial | no |
| Brave Answer / Leo | none | no | no | no | n/a | no |
| Kagi FastGPT | none | no | no | snippets only | n/a | no |
| CRAG (paper) | none | yes | yes | yes | unified evaluator | n/a |
| Self-RAG (paper) | none | yes | yes | n/a | reflection tokens | n/a |
| FLARE (paper) | none | yes (confidence-based) | yes | n/a | unified | n/a |
| Adaptive-RAG (paper) | 3-branch | per-branch | multi-hop only | n/a | no | n/a |
| **Thuki `/search`** | **2-branch (clarify / proceed)** | **yes (universal)** | **yes (bounded ≤ 3)** | **yes (Trafilatura)** | **single loop** | **yes** |

The shape matches the state-of-the-art cloud products and the relevant academic literature almost exactly. The distinguishing feature is the last column: `/search` runs on the user's machine, end to end.

The "single unified loop with one judge" design (as opposed to "3-branch router with per-branch gates") is what CRAG, Self-RAG, OpenAI Deep Research, and Exa Observer all picked in 2024-2025 as they converged. Adaptive-RAG is the paper from the prior era that kept per-branch gates; it has fallen out of favor because the unified loop is simpler to reason about and simpler to test.

---

## 16. Configuration knobs

All tuning parameters live in one place: `src-tauri/src/search/config.rs`. They are compile-time constants, not user-configurable. Tuning requires a rebuild, which is intentional: downstream prompt design and persisted metadata interpretation assume these exact values, and we do not want a user to accidentally push `MAX_ITERATIONS` to 99 and wait forever.

| Constant | Value | What it controls |
|---|---|---|
| `MAX_ITERATIONS` | 3 | Hard cap on the gap loop (initial + 2 gap rounds) |
| `GAP_QUERIES_PER_ROUND` | 3 | Upper bound on gap queries the judge can emit per round |
| `TOP_K_URLS` | 10 | Reranked URLs forwarded to reader and judge |
| `CHUNK_TOKEN_SIZE` | 500 | Approximate word budget per reader-content chunk |
| `TOP_K_CHUNKS` | 8 | Top-scoring chunks passed to the synthesis prompt |
| `LLM_RETRY_DELAY_MS` | 500 | Backoff before the single LLM retry on transient failure |
| `SEARCH_RETRY_DELAY_MS` | 1000 | Backoff before the single SearXNG retry |
| `READER_RETRY_DELAY_MS` | 500 | Backoff before the single reader retry |
| `ROUTER_TIMEOUT_S` | 45 | Router LLM call budget (generous for cold-start models) |
| `SEARCH_TIMEOUT_S` | 20 | SearXNG call budget |
| `READER_PER_URL_TIMEOUT_S` | 10 | Per-URL reader fetch budget |
| `READER_BATCH_TIMEOUT_S` | 30 | Whole-batch reader budget |
| `JUDGE_TIMEOUT_S` | 30 | Judge LLM call budget |
| `READER_BASE_URL` | `http://127.0.0.1:25018` | Hardcoded reader sidecar URL |

---

## 17. Performance characteristics

On a local machine with the default Gemma 4 e2b model, the typical `/search` takes:

| Path | Typical duration | Bottleneck |
|---|---|---|
| Clarify | 1 to 3 seconds | Single LLM round-trip |
| History-sufficient shortcut | 3 to 8 seconds | Synthesis LLM tokens streamed |
| Snippets sufficient | 8 to 15 seconds | SearXNG (1 to 2s) + snippet judge + synthesis |
| One reader escalation | 12 to 25 seconds | Reader batch (3 to 8s) + chunk judge + synthesis |
| One gap round | adds 10 to 20 seconds | Extra SearXNG fanout + reader + judge |
| Full exhaustion (3 rounds) | 40 to 70 seconds | 3 iterations of everything |

Cold starts add 10 to 30 seconds on the first query because the model has to load into VRAM. Subsequent queries are warm.

All stages race a cancellation token. Dismissing the overlay at any point drops all in-flight HTTP connections and stops all inference within milliseconds.

---

## 18. Future work

A few things we deliberately did not ship in v3 but left well-prepared hooks for:

- **JS rendering for SPA pages**. The reader currently returns `status: "empty"` when Trafilatura extracts nothing from a JS-only page. We log these URLs. If telemetry shows a significant fraction of user queries landing on such pages, we add a Playwright-backed rendering layer as a compose service alongside the reader. The decision is data-driven rather than speculative.

- **Dense embedding reranker**. BM25F + RRF is a strong baseline. Modern production systems (Cohere rerank-v3, BGE-reranker-v2, Jina reranker) use dense embeddings. If retrieval quality becomes a bottleneck we can swap in a sentence-transformer model as a second pass on top of BM25. The pipeline seam is clean.

- **Dedicated small judge model**. CRAG's original paper used T5-large as the critic. We currently use the same Ollama model for router, judge, and synthesis. Pulling a smaller faster model alongside the synthesis model (for example `qwen2.5:0.5b`) and routing judge calls to it would cut judge latency significantly without changing anything else.

- **Post-synthesis groundedness check**. Self-RAG ends with a verification pass that asks "is every claim in the answer supported by a cited source?" A lightweight version of this as an optional final step would catch the small-model hallucinations that still slip through.

- **Resume-from-cancellation**. Currently dismissing the overlay mid-loop drops all state. A more polished version would persist the partial iteration trace and let the user reopen the overlay to continue where the pipeline left off.

- **User-facing iteration trail**. The persisted metadata is debugging-only today. A collapsible "show me what Thuki did" panel under each answer, with the query trail and judge reasoning exposed, would be a nice power-user feature.

---

## 19. File map

Canonical locations for every piece of the pipeline.

```
src-tauri/src/search/
├── mod.rs               Tauri command entry, cancellation wiring, trait seams
├── pipeline.rs          The orchestrator: run_agentic and its helpers
├── config.rs            Compile-time tuning constants
├── llm.rs               Router + judge LLM calls, prompt-loading, synthesis message builder
├── judge.rs             Tolerant JSON extraction, verdict normalization
├── reader.rs            HTTP client for the Trafilatura sidecar
├── chunker.rs           Markdown splitting with URL preservation
├── rerank.rs            BM25F + RRF for URLs; single-field BM25 for chunks
├── searxng.rs           SearXNG HTTP client, single and parallel fanout
├── errors.rs            retry_once helper, transient error classifier
└── types.rs             Cross-IPC event enum, sufficiency, warning, metadata shapes

src-tauri/prompts/
├── search_plan.txt           Router+judge system prompt
├── search_judge.txt          Universal sufficiency judge prompt
├── search_synthesis.txt      Answer-synthesis system prompt with few-shot example
└── system_prompt.txt         Main Thuki chat system prompt (not search-specific)

sandbox/search-box/
├── docker-compose.yml        Defines both searxng and reader services
├── searxng/
│   ├── README.md
│   └── settings.yml
└── reader/
    ├── README.md
    ├── Dockerfile
    ├── main.py
    ├── requirements.txt
    ├── requirements-dev.txt
    └── test_main.py

sandbox/llm-box/
└── docker-compose.yml        Ollama runtime

src/
├── types/search.ts           TS mirror of SearchEvent, SearchWarning, SearchStage
├── hooks/useOllama.ts        askSearch() Channel handler, warning accumulation
├── view/ConversationView.tsx searchStageLabel helper
├── components/
│   ├── LoadingStage.tsx      Renders the current stage label
│   ├── SearchWarningIcon.tsx Amber/red icon beside Sources footer
│   ├── Tooltip.tsx           Shared animated tooltip (supports multiline)
│   └── ChatBubble.tsx        Renders the assistant bubble, Sources, warning icon
└── config/searchWarnings.ts  Enum → friendly copy, enum → severity maps

docs/
└── agentic-search.md         This document
```

---

## 20. Reading list

If you want to go deeper into the ideas behind each stage:

- **BM25 and BM25F**: Robertson, S. E. et al. (2009). *The Probabilistic Relevance Framework: BM25 and Beyond*. Still the canonical reference.
- **Reciprocal Rank Fusion**: Cormack, G. V. et al. (2009). *Reciprocal Rank Fusion outperforms Condorcet and individual rank learning methods*. The seven-page SIGIR paper that introduced RRF.
- **CRAG**: Yan, S. et al. (2024). *Corrective Retrieval Augmented Generation*. arXiv:2401.15884. The unified-evaluator reference.
- **Self-RAG**: Asai, A. et al. (2023). *Self-RAG: Learning to Retrieve, Generate, and Critique through Self-Reflection*. Reflection-token design.
- **FLARE**: Jiang, Z. et al. (2023). *Active Retrieval Augmented Generation*. arXiv:2305.06983. Generation-confidence-triggered retrieval.
- **Adaptive-RAG**: Jeong, S. et al. (2024). *Adaptive-RAG: Learning to Adapt Retrieval-Augmented Large Language Models through Question Complexity*. The per-branch-gate design.
- **ReAct**: Yao, S. et al. (2022). *ReAct: Synergizing Reasoning and Acting in Language Models*. arXiv:2210.03629. The foundational reasoning-plus-tool-use pattern.
- **Trafilatura**: Barbaresi, A. (2021). *Trafilatura: A Web Scraping Library and Command-Line Tool for Text Discovery and Extraction*. ACL system demonstration. The only extractor to win the ScrapingHub benchmark.

That is everything. If after reading this paper you understand what Thuki is doing on each keystroke, what each box in the flow diagram actually computes, and why the design looks the way it does, the paper did its job.
