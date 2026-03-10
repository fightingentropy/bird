#!/usr/bin/env bash

set -euo pipefail

BIRD_BIN="${BIRD_BIN:-bird}"
SEARCH_QUERY="${BIRD_SEARCH_QUERY:-from:MoonOverlord}"
READ_ID="${BIRD_READ_ID:-2031456653053218817}"
WRITE_SMOKE="${BIRD_WRITE_SMOKE:-0}"

run_cmd() {
  echo
  echo "+ $*"
  "$@"
}

run_cmd "$BIRD_BIN" check
run_cmd "$BIRD_BIN" whoami
run_cmd "$BIRD_BIN" query-ids --fresh --json
run_cmd "$BIRD_BIN" home -n 1 --json
run_cmd "$BIRD_BIN" search "$SEARCH_QUERY" -n 1 --json
run_cmd "$BIRD_BIN" read "$READ_ID" --json
run_cmd "$BIRD_BIN" bookmarks -n 1 --json
run_cmd "$BIRD_BIN" news -n 1 --json

if [[ "$WRITE_SMOKE" != "1" ]]; then
  echo
  echo "Write smoke tests skipped. Set BIRD_WRITE_SMOKE=1 to enable explicit side-effect checks."
  exit 0
fi

echo
echo "Write smoke tests are enabled and may create or modify live state."

if [[ -n "${BIRD_SMOKE_TWEET_TEXT:-}" ]]; then
  run_cmd "$BIRD_BIN" tweet "${BIRD_SMOKE_TWEET_TEXT}"
fi

if [[ -n "${BIRD_SMOKE_REPLY_TO:-}" && -n "${BIRD_SMOKE_REPLY_TEXT:-}" ]]; then
  run_cmd "$BIRD_BIN" reply "${BIRD_SMOKE_REPLY_TO}" "${BIRD_SMOKE_REPLY_TEXT}"
fi

if [[ -n "${BIRD_SMOKE_UNBOOKMARK_ID:-}" ]]; then
  run_cmd "$BIRD_BIN" unbookmark "${BIRD_SMOKE_UNBOOKMARK_ID}"
fi

if [[ -n "${BIRD_SMOKE_FOLLOW_TARGET:-}" ]]; then
  run_cmd "$BIRD_BIN" follow "${BIRD_SMOKE_FOLLOW_TARGET}"
fi

if [[ -n "${BIRD_SMOKE_UNFOLLOW_TARGET:-}" ]]; then
  run_cmd "$BIRD_BIN" unfollow "${BIRD_SMOKE_UNFOLLOW_TARGET}"
fi
