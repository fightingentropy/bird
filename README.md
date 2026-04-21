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
- Native macOS `libcurl-impersonate` integration for stronger browser fingerprint parity

## Install

### One-command install

```bash
curl -fsSL https://raw.githubusercontent.com/fightingentropy/bird/main/scripts/install.sh | bash
```

By default that installs `bird` and `sweet-cookie-diagnose` into `~/.local/bin`.

Useful overrides:

- `BIRD_INSTALL_DIR=/usr/local/bin`
- `BIRD_VERSION=v0.1.2`
- `BIRD_GITHUB_REPO=fightingentropy/bird`

Current installer/release targets: macOS Apple Silicon and Linux x64.

### From a release archive

Download the matching release archive for your platform, extract it, and place `bird` somewhere on your `PATH`.

```bash
tar -xzf bird-v0.1.2-aarch64-apple-darwin.tar.gz
install -m 755 bird-v0.1.2-aarch64-apple-darwin/bin/bird /usr/local/bin/bird
```

Linux x64 uses the corresponding archive name:

```bash
tar -xzf bird-v0.1.2-x86_64-unknown-linux-gnu.tar.gz
install -m 755 bird-v0.1.2-x86_64-unknown-linux-gnu/bin/bird /usr/local/bin/bird
```

The release archive also includes `sweet-cookie-diagnose`, a small troubleshooting binary for cookie inspection.

### From source

```bash
git clone https://github.com/fightingentropy/bird.git
cd bird
cargo build --locked --release -p bird-cli
install -m 755 target/release/bird /usr/local/bin/bird
```

On macOS, the native impersonation build expects these tools on `PATH`:

```bash
brew install pkg-config make cmake ninja go autoconf automake libtool
```

The repo vendors the macOS source archives needed by `libcurl-impersonate` under `third_party/curl-impersonate/distfiles/`, so the native build is network-free after clone. The remaining helpers used by the vendored transport (`patch`, `tar`, `unzip`) are available in a standard macOS install.

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
bird transport
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

On native macOS builds, `bird` links a vendored `libcurl-impersonate` and applies it automatically for X/Twitter hosts while keeping request headers under `bird`'s control. The vendored build is cached under `target/curl-impersonate-cache/`.

Inspect the active transport configuration without making a network request:

```bash
bird transport
bird transport --json
```

You can override the native impersonation profile at runtime:

```bash
export BIRD_CURL_IMPERSONATE=chrome136
bird home -n 5
```

Non-macOS builds keep the plain libcurl transport path.

## Platform notes

- macOS Apple Silicon and Linux x64 are the supported release targets
- macOS Apple Silicon builds include the native impersonation transport
- Linux x64 builds use the plain libcurl transport path
- Safari cookie support is included on macOS
- Chrome and Firefox cookie support are included on supported platforms

## Build a release

```bash
./scripts/package-release.sh
```

That produces a versioned tarball in `dist/`.
