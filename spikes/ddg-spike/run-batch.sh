#!/usr/bin/env bash
#
# T1 self-recording harness for the DuckDuckGo durability spike.
#
# Runs every query in a query file through ddg-spike, spacing requests out with
# a randomized human-like delay so a batch never trips DuckDuckGo's burst
# anomaly detection (empirically: ~4 rapid requests trips a multi-hour block).
# Every outcome is appended to the JSONL log by the binary itself, so the data
# is consistent and lossless across runs and days.
#
# Usage:
#   ./run-batch.sh [network_label] [queries_file] [min_gap_s] [max_gap_s]
#
# Defaults: network "home", queries.txt, gap 30-75s. Locale rotates across a
# small set so the log captures locale variance. Safe to run daily (append) and
# from cron.
set -euo pipefail

cd "$(dirname "$0")"

NETWORK="${1:-home}"
QUERIES_FILE="${2:-queries.txt}"
MIN_GAP="${3:-30}"
MAX_GAP="${4:-75}"
BIN="./target/release/ddg-spike"
LOCALES=("us-en" "us-en" "uk-en" "fr-fr" "de-de") # weighted toward us-en

if [[ ! -x "$BIN" ]]; then
  echo "error: $BIN not built. Run: cargo build --release" >&2
  exit 1
fi
if [[ ! -f "$QUERIES_FILE" ]]; then
  echo "error: queries file '$QUERIES_FILE' not found" >&2
  exit 1
fi

# Collect non-comment, non-blank queries (portable to bash 3.2 on macOS).
QUERIES=()
while IFS= read -r line; do
  [[ -z "${line//[[:space:]]/}" ]] && continue # skip blank / whitespace-only
  [[ "${line#"${line%%[![:space:]]*}"}" == \#* ]] && continue # skip comments
  QUERIES+=("$line")
done <"$QUERIES_FILE"
total=${#QUERIES[@]}
if [[ $total -eq 0 ]]; then
  echo "error: no queries in '$QUERIES_FILE'" >&2
  exit 1
fi

echo "== ddg-spike batch: $total queries, network=$NETWORK, gap ${MIN_GAP}-${MAX_GAP}s =="
i=0
for q in "${QUERIES[@]}"; do
  i=$((i + 1))
  locale="${LOCALES[$((RANDOM % ${#LOCALES[@]}))]}"
  echo "[$i/$total] ($locale) $q"
  # Never abort the batch on a single query's non-zero exit (a block is data).
  "$BIN" --network "$NETWORK" --locale "$locale" "$q" || true
  if [[ $i -lt $total ]]; then
    gap=$((MIN_GAP + RANDOM % (MAX_GAP - MIN_GAP + 1)))
    sleep "$gap"
  fi
done

echo
echo "== outcome tally (this log to date) =="
if command -v jq >/dev/null 2>&1; then
  jq -s 'group_by(.status) | map({status: .[0].status, count: length})' ddg-spike-log.jsonl
else
  echo "(install jq for a tally; raw log at ddg-spike-log.jsonl)"
fi
