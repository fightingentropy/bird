#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(awk -F'"' '/^version = / {print $2; exit}' "$ROOT_DIR/Cargo.toml")"
TARGET_TRIPLE="$(rustc -Vv | awk '/^host:/ { print $2 }')"
INSTALL_ROOT="${BIRD_INSTALL_ROOT:-$HOME/.local/share/bird-rust/releases}"
STATE_ROOT="${BIRD_STATE_ROOT:-$HOME/.local/share/bird-rust/state}"
TIMESTAMP="$(date -u +"%Y%m%dT%H%M%SZ")"
LINK_PATH="${BIRD_LINK_PATH:-$(command -v bird 2>/dev/null || printf '%s' "$HOME/.local/bin/bird")}"
INSTALL_DIR="$INSTALL_ROOT/${VERSION}-${TARGET_TRIPLE}"
PREVIOUS_KIND=""
PREVIOUS_TARGET=""
PREVIOUS_BACKUP=""

mkdir -p "$INSTALL_DIR" "$STATE_ROOT/backups" "$(dirname "$LINK_PATH")"

cargo build --locked --release -p bird-cli --bin bird -p sweet-cookie --bin diagnose

install -m 755 "$ROOT_DIR/target/release/bird" "$INSTALL_DIR/bird"
install -m 755 "$ROOT_DIR/target/release/diagnose" "$INSTALL_DIR/sweet-cookie-diagnose"

if [[ -L "$LINK_PATH" ]]; then
  PREVIOUS_KIND="symlink"
  PREVIOUS_TARGET="$(readlink "$LINK_PATH")"
elif [[ -f "$LINK_PATH" ]]; then
  PREVIOUS_KIND="file"
  PREVIOUS_BACKUP="$STATE_ROOT/backups/bird-${TIMESTAMP}"
  cp "$LINK_PATH" "$PREVIOUS_BACKUP"
fi

rm -f "$LINK_PATH"
ln -s "$INSTALL_DIR/bird" "$LINK_PATH"

STATE_FILE="$STATE_ROOT/current-install.env"
{
  printf 'installed_at=%q\n' "$TIMESTAMP"
  printf 'version=%q\n' "$VERSION"
  printf 'target_triple=%q\n' "$TARGET_TRIPLE"
  printf 'link_path=%q\n' "$LINK_PATH"
  printf 'rust_binary=%q\n' "$INSTALL_DIR/bird"
  printf 'diagnose_binary=%q\n' "$INSTALL_DIR/sweet-cookie-diagnose"
  printf 'previous_kind=%q\n' "$PREVIOUS_KIND"
  printf 'previous_target=%q\n' "$PREVIOUS_TARGET"
  printf 'previous_backup=%q\n' "$PREVIOUS_BACKUP"
} > "$STATE_FILE"

cp "$STATE_FILE" "$STATE_ROOT/backups/install-${TIMESTAMP}.env"

echo "Active bird command switched to Rust binary."
echo "  link:     $LINK_PATH"
echo "  target:   $INSTALL_DIR/bird"
echo "  rollback: $ROOT_DIR/scripts/rollback-rust-bird.sh"
