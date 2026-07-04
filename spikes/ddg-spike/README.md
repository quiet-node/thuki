# ddg-spike (T1, throwaway)

Week-long durability probe for the keyless DuckDuckGo HTML endpoints, per "The Assignment" in the search-revamp design doc. Not production code: no SSRF guard, no rotation, no tests. It exists only to produce real failure data before pipeline code is built.

## Build

```bash
cd spikes/ddg-spike
cargo build --release
```

## Run

Use it for your real daily queries instead of a browser search, for about a week:

```bash
./target/release/ddg-spike --network home "llama.cpp grammar sampling"
./target/release/ddg-spike --network vpn --locale fr-fr "meteo paris demain"
./target/release/ddg-spike --network cafe --endpoint lite "swift 6 concurrency changes"
```

Flags:

| Flag | Values | Default |
| --- | --- | --- |
| `--endpoint` | `html`, `lite`, `both` | `both` |
| `--network` | free-form label: `home`, `vpn`, `cafe`, `corporate` | `unlabeled` |
| `--locale` | DDG `kl` region code (`us-en`, `fr-fr`, `de-de`, ...) | `us-en` |
| `--log` | JSONL log path | `ddg-spike-log.jsonl` |

Exit code is non-zero when any probed endpoint did not return usable results, so you can notice bad runs in the shell.

## What it logs

One JSONL record per endpoint per run: timestamp, endpoint, query, network label, locale, outcome (`ok` / `rate_limited` / `captcha` / `empty` / `http_error` / `transport_error`), HTTP status, latency, result count, and the top 3 (title, resolved URL) pairs for eyeballing quality. DDG's `uddg` redirect wrappers are decoded to real URLs.

## Analyzing the week

```bash
jq -s 'group_by(.status) | map({status: .[0].status, count: length})' ddg-spike-log.jsonl
jq -s 'group_by(.network) | map({network: .[0].network, fail: (map(select(.status != "ok")) | length), total: length})' ddg-spike-log.jsonl
```

Decision input for the design: failure rate per endpoint/network/locale, whether CAPTCHA challenges ever appear at single-user volume, and whether result quality on the real query mix is acceptable.
