# bird

Fast X/Twitter CLI in Rust.

`bird` reads your existing browser session, talks to X with browser-like requests, and gives you a native CLI for timelines, tweets, profiles, lists, and common account actions.

## Highlights

- Native Rust CLI
- Safari-first cookie resolution on macOS
- Supports Safari, Chrome, and Firefox cookies
- Read and write commands
- JSON output for scripting
- Media upload support
- Built-in `curl-impersonate` support with auto-detection for stronger browser fingerprint parity

## Install

### From a release archive

Download a release archive for your platform, extract it, and place `bird` somewhere on your `PATH`.

```bash
tar -xzf bird-v0.1.0-aarch64-apple-darwin.tar.gz
install -m 755 bird-v0.1.0-aarch64-apple-darwin/bin/bird /usr/local/bin/bird
```

The release archive also includes `sweet-cookie-diagnose`, a small troubleshooting binary for cookie inspection.

### From source

```bash
git clone https://github.com/fightingentropy/bird-rs.git
cd bird-rs
cargo build --locked --release -p bird-cli
install -m 755 target/release/bird /usr/local/bin/bird
```

## Authentication

`bird` resolves credentials in this order:

1. `--auth-token` and `--ct0`
2. `AUTH_TOKEN` / `TWITTER_AUTH_TOKEN` and `CT0` / `TWITTER_CT0`
3. cached verified cookies
4. browser cookies

Default browser order on macOS:

1. Safari
2. Chrome
3. Firefox

Quick check:

```bash
bird check
bird whoami
```

If you are already logged into `x.com` in Safari, that is usually enough.

## Usage

### Read commands

```bash
bird home -n 5
bird home --following -n 5
bird search "from:elonmusk" -n 10
bird read 1234567890123456789
bird replies 1234567890123456789
bird thread 1234567890123456789
bird mentions -n 10
bird user-tweets jack -n 20
bird likes -n 20 --json
bird bookmarks -n 20
bird about nasa --json
bird lists
bird list-timeline 123456789012345678 -n 20
bird news -n 10
bird query-ids --fresh --json
```

`bird <tweet-id-or-url>` also works as shorthand for `bird read ...`.

### Write commands

```bash
bird tweet "hello from bird"
bird reply 1234567890123456789 "reply from bird"
bird tweet "photo post" --media ./photo.jpg --alt "alt text"
bird follow MoonOverlord
bird unfollow MoonOverlord
bird unbookmark 1234567890123456789
```

## Output modes

- Default output is human-readable terminal output
- `--json` prints structured JSON
- `--json-full` requests richer API payloads on supported read commands
- `--plain`, `--no-emoji`, and `--no-color` reduce formatting

## Config

`bird` reads these config files:

- `~/.config/bird/config.json5`
- `./.birdrc.json5`

Local config overrides global config.

Example:

```json5
{
  cookieSource: ["safari", "chrome", "firefox"],
  timeoutMs: 30000,
  cookieTimeoutMs: 30000,
  quoteDepth: 3,
  chromeProfile: "Default"
}
```

Useful flags:

- `--cookie-source safari|chrome|firefox`
- `--chrome-profile <name>`
- `--chrome-profile-dir <path>`
- `--firefox-profile <path>`
- `--timeout <ms>`
- `--cookie-timeout <ms>`
- `--quote-depth <n>`

Useful environment variables:

- `AUTH_TOKEN`
- `CT0`
- `TWITTER_AUTH_TOKEN`
- `TWITTER_CT0`
- `BIRD_TIMEOUT_MS`
- `BIRD_COOKIE_TIMEOUT_MS`
- `BIRD_QUOTE_DEPTH`
- `TWITTER_PROXY`

## Transport

By default, `bird` uses libcurl with HTTP/2 and compressed responses enabled.

For best X/Twitter parity, install `curl-impersonate`. When an impersonation-capable curl binary is available, `bird` detects it automatically and uses it for X/Twitter hosts. You can also set one explicitly:

```bash
export BIRD_CURL_BIN=/opt/homebrew/bin/curl-impersonate-chrome
bird home -n 5
```

If your curl binary supports `--impersonate`, you can also set:

```bash
export BIRD_CURL_IMPERSONATE=chrome136
```

## Platform notes

- macOS is the primary target
- Safari cookie support is included
- Chrome and Firefox cookie support are included
- Linux and Windows are not the primary support target yet

## Build a release

```bash
./scripts/package-release.sh
```

That produces a versioned tarball in `dist/`.
