# SearXNG service

Self-hosted, privacy-respecting meta-search engine. First stop of Thuki's `/search` pipeline.

## What it does

SearXNG aggregates results from 70+ upstream engines (Google, Bing, DuckDuckGo, Brave, Wikipedia, Stack Overflow, GitHub, arXiv, etc.) and returns a unified JSON result set. Our instance is:

- **Local-only**: bound to `127.0.0.1:25017`, never reachable from the network.
- **No tracking**: SearXNG does not log queries, rotate fingerprints, or rate-limit (rate limiting is intentionally disabled for local performance; localhost binding is the abuse mitigation).
- **No API key**: no account, no signup, no upstream dependency on a paid provider.

## Why Thuki needs it

The agentic `/search` pipeline needs fresh web results every time the router or judge decides the conversation history is insufficient. Options considered:

- **Bing / Google Custom Search APIs**: paid, require user accounts, phone home every query (kills the privacy-first posture).
- **Scrape search engines directly from Rust**: fragile, blocked by every major engine within days.
- **SearXNG**: drop-in meta-search that normalizes N upstream engines into one JSON endpoint, runs entirely locally, no vendor relationship.

SearXNG gives us freshness + privacy + zero cost in one container.

## How it fits into the pipeline

```
user query
  -> router LLM (decides whether to search)
    -> SearXNG GET /search?q=<query>&format=json
      -> BM25F + RRF rerank (Rust)
        -> top URLs
          -> snippets judge OR reader escalation
            -> synthesis LLM
              -> streamed answer with citations
```

SearXNG returns URLs, titles, and short snippet content for each result. The Rust side reranks and (when the judge rules snippets insufficient) hands the top URLs to the reader sidecar for full-page extraction.

## Architecture

```
Thuki (Tauri) ─── http://127.0.0.1:25017/search?q=... ───▶ SearXNG
                                                            │
                                                            ▼
                                                      upstream engines
                                                      (rotated, HTTPS)
```

Single container, single port, no persistence (no database, no cache volume). Every query flows through the SearXNG image at `searxng/searxng:latest`.

## Security posture

Defined in `sandbox/search-box/docker-compose.yml`:

- **Network ingress**: `127.0.0.1:25017` only. External hosts cannot reach the service.
- **Privilege escalation**: `no-new-privileges:true` stops child processes from gaining capabilities.
- **Capability restriction**: `cap_drop: ALL`, `cap_add` limited to `CHOWN`, `SETGID`, `SETUID` (required by uwsgi init).
- **Rate limiting**: intentionally disabled for local performance. Localhost binding is the abuse mitigation.
- **Network**: shares the `search_net` bridge with the reader service; both remain host-isolated.

## Configuration

`settings.yml` in this directory overrides the upstream defaults. Edit there for:

- Default engine set (which upstreams to query)
- Results cap (we keep SearXNG's default; the pipeline truncates to `TOP_K_URLS=5` in Rust)
- Safe search level
- Preferred language

Never commit API keys or credentials here; this project has none and does not need them.

## Local development

```bash
bun run sandbox:start     # brings up searxng + reader + pulls Ollama model
bun run sandbox:stop      # tears down and wipes volumes

# Hit it directly for debugging:
curl -sS 'http://127.0.0.1:25017/search?q=hello&format=json' | jq '.results[0:3]'
```

Confirm the container is healthy:

```bash
docker compose -f sandbox/search-box/docker-compose.yml ps
```

## What SearXNG is not

- Not a general-purpose web server. The Tauri frontend never calls it directly; all traffic goes through the Rust `search` module.
- Not a cache. Every query hits upstream engines fresh. If you want caching, add it in Rust (we currently don't).
- Not a ranker. SearXNG returns upstream-engine ordering; our Rust BM25F+RRF reranker is the real ranking step.
