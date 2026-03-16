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
./target/release/bird transport --json
BIRD_BIN=./target/release/bird ./scripts/live-smoke.sh
```

For transport experiments:

```bash
BIRD_CURL_IMPERSONATE=chrome136 BIRD_BIN=./target/release/bird ./scripts/live-smoke.sh
```

On native macOS builds, `bird` links the vendored impersonation-capable libcurl directly. You can override the profile with:

```bash
BIRD_CURL_IMPERSONATE=chrome145 bird transport --json
BIRD_CURL_IMPERSONATE=chrome145 bird whoami
```

On this machine, March 16, 2026:

- the Rust build links vendored `libcurl-impersonate` on macOS
- the default runtime profile is `chrome136`
- the vendored macOS build cache lives under `target/curl-impersonate-cache/`

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
