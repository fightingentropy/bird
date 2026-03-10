# bird-workspace

Native Rust rewrite of `sweet-cookie` and `bird`.

## Layout

- `crates/sweet-cookie`: browser cookie extraction library plus diagnostic binary
- `crates/bird-core`: auth, transport, query-id cache, and Twitter/X client logic
- `crates/bird-cli`: user-facing CLI compatible with the current JS `bird`

## Current Status

- JS parity baseline captured in [`docs/parity.md`](/Users/erlinhoxha/Developer/bird-workspace/docs/parity.md)
- Rust packaging, live-smoke, cutover, and rollback instructions live in [`docs/cutover.md`](/Users/erlinhoxha/Developer/bird-workspace/docs/cutover.md)
- `sweet-cookie` is implemented with Safari, Chromium, Firefox, and inline cookie sources
- `bird-core` resolves credentials, verifies cookies, caches query IDs, generates native `x-client-transaction-id` headers, and serves the authenticated read stack over a tuned libcurl transport with optional external curl impersonation support
- `bird` currently supports: `check`, `whoami`, `query-ids`, `tweet`, `reply`, `unbookmark`, `follow`, `unfollow`, `likes`, `bookmarks`, `following`, `followers`, `about`, `lists`, `list-timeline`, `news`/`trending`, `home`, `read`, `replies`, `thread`, `search`, `mentions`, `user-tweets`, plus `bird <tweet-id-or-url>` shorthand
- Text tweet/reply, media upload, bookmark removal, follow/unfollow, bookmark timelines, news/trending, the main account/list read commands, release packaging, and cutover tooling are implemented; destructive live write validation remains opt-in because it mutates real account state

## Running

```bash
cargo run -p bird-cli -- check
cargo run -p bird-cli -- whoami
cargo run -p bird-cli -- tweet "hello from rust bird"
cargo run -p bird-cli -- follow @example
cargo run -p bird-cli -- likes -n 5 --json
cargo run -p bird-cli -- bookmarks -n 5 --json
cargo run -p bird-cli -- following -n 20 --json
cargo run -p bird-cli -- about @example --json
cargo run -p bird-cli -- news -n 5 --json
cargo run -p bird-cli -- home -n 5 --json
cargo run -p bird-cli -- search "from:MoonOverlord" -n 3 --json
./scripts/package-release.sh
BIRD_BIN=./target/release/bird ./scripts/live-smoke.sh
./scripts/cutover-rust-bird.sh
```

## Transport Notes

- Default transport: in-process libcurl with HTTP/2 and compressed-response support enabled
- Auto-detected external transport: if `curl-impersonate-chrome` or other supported binaries are on `PATH`, `bird` will prefer them automatically for X/Twitter hosts
- Optional external transport: set `BIRD_CURL_BIN` to a curl-compatible binary to route X/Twitter traffic through that executable
- Optional impersonation flag: set `BIRD_CURL_IMPERSONATE` when the configured binary supports `--impersonate`
- `curl-impersonate-chrome` receives Chrome transport-profile flags directly from `bird`, so Rust keeps control of auth headers while the external binary supplies the browser-like TLS/HTTP2 behavior

## Reference Snapshots

These repos remain the current JS references and should not be edited as part of the rewrite:

- `/Users/erlinhoxha/Developer/sweet-cookie`
- `/Users/erlinhoxha/Developer/bird`
