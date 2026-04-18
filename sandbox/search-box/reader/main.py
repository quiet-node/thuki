from __future__ import annotations

import ipaddress
from urllib.parse import urlparse

import httpx
import trafilatura
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel, Field

app = FastAPI(title="thuki-reader", version="1.0.0")

FETCH_TIMEOUT_SECONDS = 8.0
MAX_BYTES = 2_000_000


class ExtractRequest(BaseModel):
    url: str = Field(..., min_length=1, max_length=2048)


class ExtractResponse(BaseModel):
    url: str
    title: str
    markdown: str
    status: str  # "ok" | "empty"


def _is_private_host(host: str) -> bool:
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        return host in {"localhost"}
    return ip.is_loopback or ip.is_private or ip.is_link_local or ip.is_multicast or ip.is_reserved


def _validate_url(url: str) -> None:
    parsed = urlparse(url)
    if parsed.scheme not in {"http", "https"}:
        raise HTTPException(status_code=400, detail="unsupported_scheme")
    if not parsed.hostname:
        raise HTTPException(status_code=400, detail="missing_host")
    if _is_private_host(parsed.hostname):
        raise HTTPException(status_code=400, detail="private_host_blocked")


def fetch_html(url: str) -> str:
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
    return {"status": "ok"}
