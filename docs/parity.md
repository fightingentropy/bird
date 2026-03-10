# JS Parity Baseline

Captured on March 10, 2026 from the local JS reference snapshots:

- `/Users/erlinhoxha/Developer/sweet-cookie`
- `/Users/erlinhoxha/Developer/bird`

## Credential Resolution

- `bird check` passes with Safari as the resolved source.
- `sweet-cookie` resolves Safari first for `https://x.com/` with `auth_token` and `ct0`.
- The current resolved X/Twitter cookie header contains 9 cookies:
  - `__cuid`
  - `auth_token`
  - `ct0`
  - `d_prefs`
  - `dnt`
  - `g_state`
  - `guest_id`
  - `kdt`
  - `twid`

## Check Output

```text
[info] Credential check
────────────────────────────────────────
[ok] auth_token: 293ec5792d...
[ok] ct0: 052d519fb7...
source: Safari

[ok] Ready to tweet!
```

## Whoami Output

```text
source: Safari
user: @Inagitabilis (Inagitabilis)
user_id: 2470753483
engine: graphql
credentials: Safari
```

## Query ID Cache

- Cache path: `~/.config/bird/query-ids-cache.json`
- Captured JS cache was stale on March 10, 2026 and still used as fallback.
- Example entries present:
  - `HomeTimeline`: `XzjVq_S9RnjdhmUGGPjpuw`
  - `HomeLatestTimeline`: `ZibLTUqUvOqCmyVWrey-GA`
  - `SearchTimeline`: `f_A-Gyo204PRxixpkrchJg`
  - `TweetDetail`: `Kzfv17rukSzjT96BerOWZA`
  - `UserTweets`: `a3SQAz_VP9k8VWDr9bMcXQ`

## Representative JSON Outputs

### `bird home -n 1 --json`

```json
[
  {
    "id": "2031456653053218817",
    "text": "RT @loomdart: @MoonOverlord Btc 100k spx 6500 wahooooo",
    "createdAt": "Tue Mar 10 19:46:23 +0000 2026",
    "replyCount": 0,
    "retweetCount": 10,
    "likeCount": 0,
    "conversationId": "2031456653053218817",
    "author": {
      "username": "MoonOverlord",
      "name": "moon"
    },
    "authorId": "938606626017370112"
  }
]
```

### `bird read 2031456653053218817 --json`

```json
{
  "id": "2031456653053218817",
  "text": "RT @loomdart: @MoonOverlord Btc 100k spx 6500 wahooooo",
  "createdAt": "Tue Mar 10 19:46:23 +0000 2026",
  "replyCount": 0,
  "retweetCount": 10,
  "likeCount": 0,
  "conversationId": "2031456653053218817",
  "author": {
    "username": "MoonOverlord",
    "name": "moon"
  },
  "authorId": "938606626017370112"
}
```

### `bird search "from:MoonOverlord" -n 1 --json`

```json
[
  {
    "id": "2031456345220698518",
    "text": "@DeepDishEnjoyer \"we have blown up all their mines to smithereens, total destruction, some are saying its the biggest mine destruction theyve ever seen\" \n\n(they have thousands more they can just dump in the canal whenever they want)",
    "createdAt": "Tue Mar 10 19:45:09 +0000 2026",
    "replyCount": 0,
    "retweetCount": 1,
    "likeCount": 14,
    "conversationId": "2031453925337964595",
    "inReplyToStatusId": "2031453925337964595",
    "author": {
      "username": "MoonOverlord",
      "name": "moon"
    },
    "authorId": "938606626017370112"
  }
]
```

## Request Metadata To Preserve

Derived from the current JS implementation:

- Auth verification hits:
  - `https://api.x.com/1.1/account/verify_credentials.json`
  - `https://x.com/i/api/1.1/account/settings.json`
- Base headers always include:
  - `authorization`
  - `cookie`
  - `x-csrf-token`
  - `x-twitter-auth-type`
  - `x-twitter-active-user`
  - `x-twitter-client-language`
  - `x-client-uuid`
  - `x-twitter-client-deviceid`
  - `user-agent`
  - `sec-ch-ua`
  - `sec-ch-ua-mobile`
  - `sec-ch-ua-platform`
  - `sec-fetch-dest`
  - `sec-fetch-mode`
  - `sec-fetch-site`
  - `priority`
  - `origin`
  - `referer`
- Current JS transport uses curl impersonation (`cuimp`) for X/Twitter hosts.
- Phase 2 fallback transaction IDs are random 16-byte hex strings when native generation is unavailable.
- Write flow fallback after GraphQL error `226` uses `statuses/update.json`.
