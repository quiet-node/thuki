from fastapi.testclient import TestClient
from main import app

client = TestClient(app)

def test_extract_returns_markdown_for_well_formed_html(monkeypatch):
    def fake_fetch(url: str) -> str:
        return "<html><head><title>T</title></head><body><article><p>Hello world.</p></article></body></html>"
    monkeypatch.setattr("main.fetch_html", fake_fetch)
    r = client.post("/extract", json={"url": "https://example.com/a"})
    assert r.status_code == 200
    body = r.json()
    assert body["url"] == "https://example.com/a"
    assert body["title"] == "T"
    assert "Hello world." in body["markdown"]
    assert body["status"] == "ok"

def test_extract_returns_empty_body_when_trafilatura_yields_nothing(monkeypatch):
    monkeypatch.setattr("main.fetch_html", lambda _url: "<html><body></body></html>")
    r = client.post("/extract", json={"url": "https://example.com/empty"})
    assert r.status_code == 200
    body = r.json()
    assert body["markdown"] == ""
    assert body["status"] == "empty"

def test_extract_handles_fetch_failure(monkeypatch):
    def raise_err(_url):
        raise RuntimeError("network")
    monkeypatch.setattr("main.fetch_html", raise_err)
    r = client.post("/extract", json={"url": "https://example.com/x"})
    assert r.status_code == 502
    assert r.json()["detail"] == "fetch_failed"

def test_extract_rejects_non_http_scheme():
    r = client.post("/extract", json={"url": "file:///etc/passwd"})
    assert r.status_code == 400

def test_extract_rejects_private_ip(monkeypatch):
    r = client.post("/extract", json={"url": "http://127.0.0.1/"})
    assert r.status_code == 400
    r = client.post("/extract", json={"url": "http://192.168.1.1/"})
    assert r.status_code == 400
