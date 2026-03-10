#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(awk -F'"' '/^version = / {print $2; exit}' "$ROOT_DIR/Cargo.toml")"
TARGET_TRIPLE="$(rustc -Vv | awk '/^host:/ { print $2 }')"
DIST_DIR="${DIST_DIR:-$ROOT_DIR/dist}"
PACKAGE_NAME="bird-v${VERSION}-${TARGET_TRIPLE}"
PACKAGE_DIR="$DIST_DIR/$PACKAGE_NAME"
ARCHIVE_PATH="$DIST_DIR/${PACKAGE_NAME}.tar.gz"
CHECKSUM_PATH="${ARCHIVE_PATH}.sha256"
BUILD_DATE="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
RUSTC_VERSION="$(rustc -V)"

mkdir -p "$DIST_DIR"
rm -rf "$PACKAGE_DIR"
mkdir -p "$PACKAGE_DIR/bin" "$PACKAGE_DIR/docs"

cargo build --locked --release -p bird-cli --bin bird -p sweet-cookie --bin diagnose

cp "$ROOT_DIR/target/release/bird" "$PACKAGE_DIR/bin/bird"
cp "$ROOT_DIR/target/release/diagnose" "$PACKAGE_DIR/bin/sweet-cookie-diagnose"
cp "$ROOT_DIR/README.md" "$PACKAGE_DIR/README.md"
cp "$ROOT_DIR/docs/parity.md" "$PACKAGE_DIR/docs/parity.md"
cp "$ROOT_DIR/docs/cutover.md" "$PACKAGE_DIR/docs/cutover.md"

cat > "$PACKAGE_DIR/BUILD-INFO.txt" <<EOF
version: $VERSION
target: $TARGET_TRIPLE
built_at_utc: $BUILD_DATE
rustc: $RUSTC_VERSION
workspace: $ROOT_DIR
EOF

rm -f "$ARCHIVE_PATH" "$CHECKSUM_PATH"
tar -C "$DIST_DIR" -czf "$ARCHIVE_PATH" "$PACKAGE_NAME"
shasum -a 256 "$ARCHIVE_PATH" > "$CHECKSUM_PATH"

echo "Created release package:"
echo "  archive: $ARCHIVE_PATH"
echo "  sha256:  $CHECKSUM_PATH"
echo "  dir:     $PACKAGE_DIR"
