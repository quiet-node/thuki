"""Trafilatura-based reader sidecar for the Thuki agentic /search pipeline.

The Rust backend hits `POST /extract` with a URL; this service fetches the page
and returns LLM-ready markdown plus the page title. A single library call to
`trafilatura.extract` does the boilerplate-stripping heavy lifting; everything
else here is transport, validation, and safety bounds.

Security posture (enforced at both container and app layer):
- SSRF guard: rejects non-http(s) schemes and private / loopback / link-local
  / multicast / reserved addresses so a malicious URL cannot reach internal
  services.
- DNS rebinding boundary: this service only validates literal hostnames and IP
    strings. It does not resolve public hostnames before connect, so DNS
    rebinding must still be contained by the localhost-only network boundary and
    container isolation.
- Byte cap: fetch aborts once ``MAX_BYTES`` is exceeded, so a hostile server
  cannot exhaust memory.
- Timeout: 8s hard ceiling on upstream fetch.
- Container hardening lives in ``sandbox/search-box/docker-compose.yml``
  (cap_drop ALL, no-new-privileges, read-only rootfs, localhost-only port).
"""

from __future__ import annotations

import ipaddress
from urllib.parse import urlparse

import httpx
import trafilatura
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel, Field

app = FastAPI(title="thuki-reader", version="1.0.0")

FETCH_TIMEOUT_SECONDS = 8.0
"""Hard ceiling on upstream fetch. The pipeline has its own per-URL timeout
above this; this value is the last line of defense inside the service."""

MAX_BYTES = 2_000_000
"""Hard cap on bytes pulled from upstream. Protects the service from hostile
servers that stream unbounded content. 2 MB easily covers a long article with
media-free HTML."""

ULA_V6_NETWORK = ipaddress.ip_network("fc00::/7")
"""IPv6 unique-local range blocked explicitly by the SSRF guard."""


class ExtractRequest(BaseModel):
    """Inbound shape for ``POST /extract``."""

    url: str = Field(..., min_length=1, max_length=2048)
    """Full URL to fetch and extract. Validated by ``_validate_url`` before
    any network call is made."""


class ExtractResponse(BaseModel):
    """Outbound shape for ``POST /extract``."""

    url: str
    """Echoes the request URL for easier client-side reconciliation."""

    title: str
    """Page title as extracted by trafilatura metadata. Empty string when the
    page has no recoverable title."""

    markdown: str
    """Extracted page body as markdown. Empty string when trafilatura finds
    no article content (typical for JS-rendered pages or paywalls)."""

    status: str
    """Either ``"ok"`` (non-empty markdown) or ``"empty"`` (no extractable
    content). The Rust caller uses this to decide whether to treat the URL as
    a reader hit or fall back to the search snippet."""


def _is_private_host(host: str) -> bool:
    """Return True if ``host`` is an address the reader must refuse to fetch.

    Covers loopback, RFC1918 private, link-local, multicast, reserved IPv4
    and IPv6 ranges, the IPv6 unique-local block ``fc00::/7``, plus the
    literal string ``"localhost"`` for paranoia. Non-IP hostnames are allowed
    through here; this service does not resolve hostnames before connect, so
    DNS-level rebinding defense remains the responsibility of the sandbox
    network boundary.
    """
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        return host in {"localhost"}
    return (
        ip.is_loopback
        or ip.is_private
        or ip.is_link_local
        or ip.is_multicast
        or ip.is_reserved
        or (ip.version == 6 and ip in ULA_V6_NETWORK)
    )


def _validate_url(url: str) -> None:
    """Raise ``HTTPException`` if the URL fails the SSRF guard.

    Three rejection cases, each with a distinct ``detail`` string for easier
    debugging:

    - ``unsupported_scheme``: anything other than http or https.
    - ``missing_host``: URL parses but has no hostname component.
    - ``private_host_blocked``: hostname is one of the ranges flagged by
      :func:`_is_private_host`.
    """
    parsed = urlparse(url)
    if parsed.scheme not in {"http", "https"}:
        raise HTTPException(status_code=400, detail="unsupported_scheme")
    if not parsed.hostname:
        raise HTTPException(status_code=400, detail="missing_host")
    if _is_private_host(parsed.hostname):
        raise HTTPException(status_code=400, detail="private_host_blocked")


def fetch_html(url: str) -> str:
    """Fetch ``url`` and return the decoded HTML body, capped at ``MAX_BYTES``.

    Streams the response so oversized bodies are truncated rather than
    buffered in full. Uses a custom User-Agent so the reader is identifiable
    in server logs. Decoding falls back to ``utf-8`` with ``errors="replace"``
    when the server does not declare an encoding.

    Raises :class:`httpx.HTTPError` on network failures; the route handler
    maps that to a 502 ``fetch_failed`` response.
    """
    with httpx.Client(follow_redirects=True, timeout=FETCH_TIMEOUT_SECONDS) as client:
        with client.stream("GET", url, headers={"User-Agent": "Thuki-Reader/1.0"}) as r:
            r.raise_for_status()
            total = 0
            chunks: list[bytes] = []
            for chunk in r.iter_bytes(chunk_size=65536):
                total += len(chunk)
                if total > MAX_BYTES:
                    break
                chunks.append(chunk)
            return b"".join(chunks).decode(r.encoding or "utf-8", errors="replace")


@app.post("/extract", response_model=ExtractResponse)
def extract(req: ExtractRequest) -> ExtractResponse:
    """Fetch ``req.url`` and return markdown + title.

    Flow: validate URL, fetch HTML (mapping network errors to 502), run
    trafilatura with ``favor_precision=True`` to prefer recall misses over
    boilerplate inclusion, read the page title from trafilatura's metadata
    pass. ``status`` is set to ``"empty"`` when trafilatura returns an empty
    body so the Rust caller can surface a warning without inspecting the
    markdown string.
    """
    _validate_url(req.url)
    try:
        html = fetch_html(req.url)
    except httpx.HTTPError:
        raise HTTPException(status_code=502, detail="fetch_failed")
    except RuntimeError:
        raise HTTPException(status_code=502, detail="fetch_failed")

    markdown = trafilatura.extract(
        html,
        output_format="markdown",
        include_comments=False,
        include_tables=True,
        favor_precision=True,
        url=req.url,
    ) or ""

    title = ""
    metadata = trafilatura.extract_metadata(html)
    if metadata is not None and metadata.title:
        title = metadata.title

    status = "ok" if markdown.strip() else "empty"
    return ExtractResponse(url=req.url, title=title, markdown=markdown, status=status)


@app.get("/healthz")
def healthz() -> dict[str, str]:
    """Liveness probe used by the docker-compose healthcheck."""
    return {"status": "ok"}
