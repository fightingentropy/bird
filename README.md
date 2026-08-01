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

Release workflows publish GitHub build-provenance attestations. If the GitHub CLI is installed,
make attestation verification mandatory during installation:

```bash
curl -fsSL https://raw.githubusercontent.com/fightingentropy/bird/main/scripts/install.sh \
  | BIRD_VERIFY_ATTESTATION=1 bash
```

Useful overrides:

- `BIRD_INSTALL_DIR=/usr/local/bin`
- `BIRD_VERSION=v0.1.4`
- `BIRD_GITHUB_REPO=fightingentropy/bird`

Current installer/release targets: macOS Apple Silicon and Linux x64.

### From a release archive

Download the matching release archive for your platform, extract it, and place `bird` somewhere on your `PATH`.

```bash
tar -xzf bird-v0.1.4-aarch64-apple-darwin.tar.gz
install -m 755 bird-v0.1.4-aarch64-apple-darwin/bin/bird /usr/local/bin/bird
```

Linux x64 uses the corresponding archive name:

```bash
tar -xzf bird-v0.1.4-x86_64-unknown-linux-gnu.tar.gz
install -m 755 bird-v0.1.4-x86_64-unknown-linux-gnu/bin/bird /usr/local/bin/bird
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

Every vendored archive has a pinned SHA-256 digest and an explicit archive root in
`manifest.json`. `scripts/vendor-supply-chain.py` verifies every entry before build and maintains
the committed SPDX SBOM at `third_party/curl-impersonate/SBOM.spdx.json`.

## Authentication

`bird` resolves credentials in this order:

1. JSON supplied over `--credentials-stdin` or an inherited `--credentials-fd`
2. `AUTH_TOKEN` / `TWITTER_AUTH_TOKEN` and `CT0` / `TWITTER_CT0`
3. cached verified cookies in macOS Keychain or Linux Secret Service
4. browser cookies

Session values are intentionally not accepted as command-line arguments. For automation, pass
this shape through stdin or a private inherited descriptor:

```json
{"authToken":"...","ct0":"..."}
```

```bash
credential-helper-that-prints-json | bird --credentials-stdin whoami

exec 9<"$XDG_RUNTIME_DIR/bird-credentials.json"
bird --credentials-fd 9 whoami
exec 9<&-
```

If the native credential store is unavailable, `bird` uses a compatibility cache only after
restricting its directory to `0700` and the cache file to `0600`. The cache stores only
`auth_token` and `ct0`, never the browser's full cookie header.

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
bird bookmarks --all --max-pages 20 --json
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

`bird` reads the user config automatically:

- `~/.config/bird/config.json5`

A project-local `./.birdrc.json5` is ignored by default because an arbitrary checkout must not
control browser-profile paths or network timeouts. Pass `--trust-project-config` for a repository
you have reviewed; the trusted local file then overrides the user config.

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
- `--trust-project-config`
- `--credentials-stdin`
- `--credentials-fd <fd>`

Useful environment variables:

- `AUTH_TOKEN`
- `CT0`
- `TWITTER_AUTH_TOKEN`
- `TWITTER_CT0`
- `BIRD_TIMEOUT_MS`
- `BIRD_COOKIE_TIMEOUT_MS`
- `BIRD_QUOTE_DEPTH`
- `TWITTER_PROXY`

When `TWITTER_PROXY` is set, `bird` prints a warning because the proxy receives authenticated X
traffic and can observe session metadata and content. Do not use a proxy you do not control or
trust.

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

Run the same format, lint, test, supply-chain, installer-safety, release-build, and smoke checks
used by CI:

```bash
bash scripts/check.sh
```

```bash
./scripts/package-release.sh
```

That produces a versioned tarball in `dist/`. The archive includes the SPDX SBOM. Tagged release
workflows attest each archive and checksum before publishing; verify a downloaded archive with:

```bash
gh attestation verify bird-v0.1.4-aarch64-apple-darwin.tar.gz \
  --repo fightingentropy/bird
```
