#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(awk -F'"' '/^version = / {print $2; exit}' "$ROOT_DIR/Cargo.toml")"
HOST_TRIPLE="$(rustc -Vv | awk '/^host:/ { print $2 }')"
TARGET_TRIPLE="${TARGET_TRIPLE:-$HOST_TRIPLE}"
DIST_DIR="${DIST_DIR:-$ROOT_DIR/dist}"
PACKAGE_NAME="bird-v${VERSION}-${TARGET_TRIPLE}"
PACKAGE_DIR="$DIST_DIR/$PACKAGE_NAME"
ARCHIVE_PATH="$DIST_DIR/${PACKAGE_NAME}.tar.gz"
CHECKSUM_PATH="${ARCHIVE_PATH}.sha256"
TARGET_RELEASE_DIR="$ROOT_DIR/target/${TARGET_TRIPLE}/release"
BUILD_DATE="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
RUSTC_VERSION="$(rustc -V)"

case "$TARGET_TRIPLE" in
  aarch64-apple-darwin|x86_64-unknown-linux-gnu)
    ;;
  *)
    echo "error: bird release packaging supports only aarch64-apple-darwin and x86_64-unknown-linux-gnu (target=${TARGET_TRIPLE}, host=${HOST_TRIPLE})" >&2
    exit 1
    ;;
esac

mkdir -p "$DIST_DIR"
rm -rf "$PACKAGE_DIR"
mkdir -p "$PACKAGE_DIR/bin"

cargo build --locked --release --target "$TARGET_TRIPLE" -p bird-cli --bin bird -p sweet-cookie --bin diagnose

cp "$TARGET_RELEASE_DIR/bird" "$PACKAGE_DIR/bin/bird"
cp "$TARGET_RELEASE_DIR/diagnose" "$PACKAGE_DIR/bin/sweet-cookie-diagnose"
cp "$ROOT_DIR/README.md" "$PACKAGE_DIR/README.md"

cat > "$PACKAGE_DIR/BUILD-INFO.txt" <<EOF
version: $VERSION
target: $TARGET_TRIPLE
built_at_utc: $BUILD_DATE
rustc: $RUSTC_VERSION
EOF

rm -f "$ARCHIVE_PATH" "$CHECKSUM_PATH"
tar -C "$DIST_DIR" -czf "$ARCHIVE_PATH" "$PACKAGE_NAME"
(
  cd "$DIST_DIR"
  shasum -a 256 "$(basename "$ARCHIVE_PATH")" > "$CHECKSUM_PATH"
)

echo "Created release package:"
echo "  archive: $ARCHIVE_PATH"
echo "  sha256:  $CHECKSUM_PATH"
echo "  dir:     $PACKAGE_DIR"
