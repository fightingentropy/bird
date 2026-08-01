#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_ROOT="$(mktemp -d)"
PACKAGE_NAME="bird-v0.0.0-test"

cleanup() {
  rm -rf "$TEST_ROOT"
}
trap cleanup EXIT

make_package() {
  local root="$1"
  mkdir -p "$root/$PACKAGE_NAME/bin"
  printf '#!/bin/sh\nexit 0\n' > "$root/$PACKAGE_NAME/bin/bird"
  printf '#!/bin/sh\nexit 0\n' > "$root/$PACKAGE_NAME/bin/sweet-cookie-diagnose"
  printf 'readme\n' > "$root/$PACKAGE_NAME/README.md"
  printf 'version: 0.0.0\n' > "$root/$PACKAGE_NAME/BUILD-INFO.txt"
  printf '{"spdxVersion":"SPDX-2.3"}\n' > "$root/$PACKAGE_NAME/SBOM.spdx.json"
  chmod 755 "$root/$PACKAGE_NAME/bin/bird" "$root/$PACKAGE_NAME/bin/sweet-cookie-diagnose"
}

SAFE_ROOT="$TEST_ROOT/safe"
make_package "$SAFE_ROOT"
tar -C "$SAFE_ROOT" -czf "$TEST_ROOT/safe.tar.gz" "$PACKAGE_NAME"
BIRD_TEST_VALIDATE_ARCHIVE=1 "$ROOT_DIR/scripts/install.sh" \
  "$TEST_ROOT/safe.tar.gz" "$PACKAGE_NAME"

EXTRA_ROOT="$TEST_ROOT/extra"
make_package "$EXTRA_ROOT"
printf 'unexpected\n' > "$EXTRA_ROOT/$PACKAGE_NAME/extra.txt"
tar -C "$EXTRA_ROOT" -czf "$TEST_ROOT/extra.tar.gz" "$PACKAGE_NAME"
if BIRD_TEST_VALIDATE_ARCHIVE=1 "$ROOT_DIR/scripts/install.sh" \
  "$TEST_ROOT/extra.tar.gz" "$PACKAGE_NAME" >/dev/null 2>&1; then
  echo "error: installer accepted an unexpected archive entry" >&2
  exit 1
fi

LINK_ROOT="$TEST_ROOT/link"
make_package "$LINK_ROOT"
rm "$LINK_ROOT/$PACKAGE_NAME/bin/bird"
ln -s /tmp/not-bird "$LINK_ROOT/$PACKAGE_NAME/bin/bird"
tar -C "$LINK_ROOT" -czf "$TEST_ROOT/link.tar.gz" "$PACKAGE_NAME"
if BIRD_TEST_VALIDATE_ARCHIVE=1 "$ROOT_DIR/scripts/install.sh" \
  "$TEST_ROOT/link.tar.gz" "$PACKAGE_NAME" >/dev/null 2>&1; then
  echo "error: installer accepted a symlink archive entry" >&2
  exit 1
fi

echo "installer archive safety tests passed"
