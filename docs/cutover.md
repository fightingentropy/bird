# Rust Cutover

As of March 10, 2026, the active JS install on this machine resolves as:

- `/opt/homebrew/bin/bird -> ../lib/node_modules/@erlinhoxha/bird/dist/cli.js`

The Rust workspace now includes the operational pieces needed to replace that command safely.

## Release Packaging

Build a release tarball with checksums:

```bash
./scripts/package-release.sh
```

This emits:

- `dist/bird-v<version>-<target>.tar.gz`
- `dist/bird-v<version>-<target>.tar.gz.sha256`
- unpacked release contents under `dist/bird-v<version>-<target>/`

## Read-Only Smoke Validation

Run the live read stack against a chosen binary:

```bash
BIRD_BIN=./target/release/bird ./scripts/live-smoke.sh
```

For transport experiments:

```bash
BIRD_CURL_BIN=/usr/bin/curl BIRD_BIN=./target/release/bird ./scripts/live-smoke.sh
```

If you install a real impersonation-capable curl build, you can point `bird` at it:

```bash
BIRD_CURL_BIN=/path/to/curl-impersonate BIRD_CURL_IMPERSONATE=chrome136 bird whoami
```

On this machine, March 10, 2026:

- `shakacode/brew/curl-impersonate` was installed successfully
- `/opt/homebrew/bin/curl-impersonate-chrome` is available
- `bird` auto-detects that binary and uses Chrome transport-profile flags for X/Twitter hosts

The default smoke run covers:

- `check`
- `whoami`
- `query-ids --fresh --json`
- `home -n 1 --json`
- `search "from:MoonOverlord" -n 1 --json`
- `read <id> --json`
- `bookmarks -n 1 --json`
- `news -n 1 --json`

Optional write checks are available, but they are gated behind `BIRD_WRITE_SMOKE=1` and explicit input env vars because they mutate live account state.

## Cutover

Install the release binary under `~/.local/share/bird-rust/releases/` and repoint the active `bird` command:

```bash
./scripts/cutover-rust-bird.sh
```

Defaults:

- install root: `~/.local/share/bird-rust/releases`
- state root: `~/.local/share/bird-rust/state`
- link path: current `command -v bird`, otherwise `~/.local/bin/bird`

The script records the prior install in `~/.local/share/bird-rust/state/current-install.env` so rollback is explicit and mechanical.

## Rollback

Restore the previous `bird` command:

```bash
./scripts/rollback-rust-bird.sh
```

If the previous install was a symlink, rollback restores that symlink target. If it was a standalone file, rollback restores the backed-up binary copy.
