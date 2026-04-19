# Reader service

Trafilatura-based URL-to-markdown extractor. Second stop of Thuki's agentic `/search` pipeline.

## What it does

Takes a URL, fetches the page, strips boilerplate (navigation, ads, footers, cookie banners), and returns clean markdown the synthesis LLM can cite against.

```
POST /extract { "url": "https://example.com/article" }
  -> { "url": "...",
       "title": "Page title",
       "markdown": "# Article text\n\nCleaned body...",
       "status": "ok" | "empty" }
```

## Why Thuki needs it

SearXNG returns URLs plus short snippets (usually the first 150-200 chars of the page). For many queries, snippets are enough. For questions like "compare tokio vs async-std benchmarks in 2026," the answer lives deep inside blog posts and docs pages that snippets never surface.

The pipeline's judge decides snippet sufficiency after the initial SearXNG round. When the judge returns `Partial` or `Insufficient`, the reader is called to fetch the top URLs in full and hand rich text to the next judge round. This is the classic "RAG reader" pattern from Perplexity, Exa, and the CRAG / Self-RAG literature.

## Why Trafilatura

HTML boilerplate removal is a surprisingly hard problem. Naive approaches (strip `<nav>`, `<footer>`) fail on modern SPAs where everything is `<div>`. Getting it right requires heuristics built over years of research. Two parallel research agents independently landed on Trafilatura as the best-in-class open-source solution:

- **F1 ~0.95** on the ScrapingHub article extraction benchmark, top of the field.
- Apache 2.0 license.
- Production use at HuggingFace, IBM, Microsoft Research, Stanford, EU Parliament.
- Pure Python, no browser, tiny attack surface.

We considered and rejected: Firecrawl (AGPL-3.0 blocks bundling), Jina Reader cloud (proxies every URL through Jina's servers, violates privacy), Crawl4AI (Chromium in container, 4 GB RAM, CVE history), ScrapeGraphAI / ReaderLM-v2 (LLM per page, wrong shape), DIY Playwright (SSRF surface without extraction value), most Rust readability crates (weaker extraction, Jan 2025 benchmark showed many return empty strings on real pages).

## How it fits into the pipeline

```
snippets judge returns Partial / Insufficient
  -> reader.fetch_batch_cancellable(&top_urls, &cancel)
    -> POST /extract for each URL in parallel (semaphore-bounded, 5 in flight)
      -> Trafilatura extraction per page
        -> chunker splits markdown into ~500-token chunks
          -> BM25 rerank picks top chunks for the query
            -> chunks judge decides sufficiency
              -> synthesis OR gap-query loop
```

The Rust `search::reader::ReaderClient` calls this service over HTTP. It races each call against a cancellation token and degrades gracefully when the reader container is unreachable (emits `ReaderUnavailable` warning, pipeline falls back to snippets).

## Architecture

Single-file FastAPI app (`main.py`, ~90 lines). One endpoint (`/extract`) and a healthz probe. Entire service fits in your head:

```
main.py
├── _validate_url        -> SSRF guard (scheme + private-host blocklist)
├── fetch_html           -> httpx stream with 8s timeout + 2MB byte cap
├── trafilatura.extract  -> boilerplate removal, markdown conversion
└── trafilatura.extract_metadata -> page title
```

The Dockerfile is standard Python-slim hardening: non-root user, minimum install, no build tools in the final layer.

## Security posture

Enforced at three layers:

**App layer (`main.py`):**
- SSRF guard rejects non-http(s) schemes plus private, loopback, link-local, multicast, and reserved IP ranges (both IPv4 and IPv6) plus the literal string `"localhost"`.
- Byte cap: upstream fetch aborts once 2 MB is buffered. Prevents hostile servers from exhausting memory.
- Timeout: 8s hard ceiling on upstream fetch.
- Request body limits (URL max length 2048 chars, validated via Pydantic).

**Container layer (`docker-compose.yml`):**
- `cap_drop: ALL` (no capabilities, not even the reduced set SearXNG needs)
- `no-new-privileges: true`
- `read_only: true` root filesystem
- `tmpfs: /tmp:size=16m` for the minimal writable scratch area
- `mem_limit: 512m`, `cpus: 1.0`
- Bound to `127.0.0.1:25018` only

**Image layer (`Dockerfile`):**
- Runs as `reader:reader` (uid/gid 10001, system user, no home directory)
- Only `main.py`, `requirements.txt`, and pinned runtime deps land in the image
- No pytest, no dev tools, no compilers

## Files in this directory

| File | Purpose | Shipped? |
|---|---|---|
| `main.py` | The service code (FastAPI app) | Yes (production) |
| `Dockerfile` | Container build recipe | Yes (production) |
| `requirements.txt` | Pinned runtime deps (6 packages) | Yes (production) |
| `requirements-dev.txt` | Pinned test deps (pytest only) | No (local-only) |
| `test_main.py` | Unit tests (5 cases) | No (local-only) |

Dev artifacts like `.venv/` and `.pytest_cache/` are gitignored and never enter the image.

## Local development

```bash
# Bring up the reader container (also pulls the image on first run):
bun run sandbox:start

# Exercise the endpoint:
curl -sS -X POST http://127.0.0.1:25018/extract \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://example.com/"}' | jq

# Healthcheck:
curl -sS http://127.0.0.1:25018/healthz

# Tear down:
bun run sandbox:stop
```

### Running pytest without Docker

```bash
cd sandbox/search-box/reader
python -m venv .venv
.venv/bin/pip install -r requirements.txt -r requirements-dev.txt
.venv/bin/python -m pytest test_main.py -v
```

`.venv/` and `.pytest_cache/` are gitignored.

## What the reader is not

- Not a browser. It does not render JavaScript. Pages that rely on client-side rendering come back as `status: "empty"`. This is tracked in pipeline telemetry; if empty-body rate gets high in production we add a Playwright fallback in v2.
- Not a crawler. One URL in, one markdown blob out. No link following, no sitemap parsing, no depth-limited traversal.
- Not a cache. Every call fetches fresh. Caching belongs upstream in the Rust pipeline if we ever need it.
- Not a general-purpose service. The endpoint accepts only http(s) URLs pointing at public hosts. Private networks and non-web schemes are rejected 400.
