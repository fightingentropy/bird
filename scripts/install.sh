#!/usr/bin/env bash

set -euo pipefail

REPO="${BIRD_GITHUB_REPO:-fightingentropy/bird}"
INSTALL_DIR="${BIRD_INSTALL_DIR:-$HOME/.local/bin}"
RELEASE_BASE_URL="${BIRD_RELEASE_BASE_URL:-}"
VERSION="${BIRD_VERSION:-}"
BINARIES="${BIRD_BINARIES:-bird,sweet-cookie-diagnose}"

log() {
  printf '%s\n' "$*"
}

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

cleanup() {
  if [[ -n "${TMP_DIR:-}" && -d "${TMP_DIR:-}" ]]; then
    rm -rf "$TMP_DIR"
  fi
}

detect_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "${os}:${arch}" in
    Darwin:arm64|Darwin:aarch64)
      printf 'aarch64-apple-darwin\n'
      ;;
    Linux:x86_64)
      printf 'x86_64-unknown-linux-gnu\n'
      ;;
    *)
      fail "unsupported platform ${os}/${arch}; bird installer supports only macOS Apple Silicon and Linux x64"
      ;;
  esac
}

require_tool() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required tool: $1"
}

http_get() {
  curl -fsSL --retry 3 --connect-timeout 15 "$1"
}

latest_release_tag() {
  http_get "https://api.github.com/repos/${REPO}/releases/latest" \
    | sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' \
    | head -n 1
}

compute_sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
    return
  fi
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
    return
  fi
  fail "missing checksum tool: need sha256sum or shasum"
}

verify_checksum() {
  local archive_path checksum_path expected actual
  archive_path="$1"
  checksum_path="$2"
  expected="$(awk 'NR==1 { print $1 }' "$checksum_path")"
  [[ -n "$expected" ]] || fail "checksum file is empty: $checksum_path"
  actual="$(compute_sha256 "$archive_path")"
  [[ "$expected" == "$actual" ]] || fail "checksum mismatch for $(basename "$archive_path")"
}

install_binary() {
  local package_dir binary source target
  package_dir="$1"
  binary="$2"
  source="${package_dir}/bin/${binary}"
  target="${INSTALL_DIR}/${binary}"
  [[ -f "$source" ]] || fail "release archive is missing ${binary}"
  install -m 755 "$source" "$target"
  log "installed ${target}"
}

print_path_hint() {
  case ":$PATH:" in
    *":${INSTALL_DIR}:"*)
      ;;
    *)
      log
      log "Add ${INSTALL_DIR} to PATH if needed:"
      log "  export PATH=\"${INSTALL_DIR}:\$PATH\""
      ;;
  esac
}

trap cleanup EXIT

require_tool curl
require_tool tar
require_tool install

TARGET="$(detect_target)"

if [[ -n "$RELEASE_BASE_URL" && -z "$VERSION" ]]; then
  fail "BIRD_VERSION is required when BIRD_RELEASE_BASE_URL is set"
fi

if [[ -z "$VERSION" ]]; then
  VERSION="$(latest_release_tag)"
fi
[[ -n "$VERSION" ]] || fail "could not resolve a release version for ${REPO}"

PACKAGE_NAME="bird-${VERSION}-${TARGET}"
ARCHIVE_NAME="${PACKAGE_NAME}.tar.gz"
CHECKSUM_NAME="${ARCHIVE_NAME}.sha256"

if [[ -n "$RELEASE_BASE_URL" ]]; then
  ARCHIVE_URL="${RELEASE_BASE_URL%/}/${ARCHIVE_NAME}"
  CHECKSUM_URL="${RELEASE_BASE_URL%/}/${CHECKSUM_NAME}"
else
  ARCHIVE_URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE_NAME}"
  CHECKSUM_URL="https://github.com/${REPO}/releases/download/${VERSION}/${CHECKSUM_NAME}"
fi

TMP_DIR="$(mktemp -d)"
ARCHIVE_PATH="${TMP_DIR}/${ARCHIVE_NAME}"
CHECKSUM_PATH="${TMP_DIR}/${CHECKSUM_NAME}"

log "Installing ${PACKAGE_NAME} from ${REPO}"
mkdir -p "$INSTALL_DIR"
[[ -w "$INSTALL_DIR" ]] || fail "install directory is not writable: ${INSTALL_DIR}"

http_get "$ARCHIVE_URL" > "$ARCHIVE_PATH" || fail "failed to download ${ARCHIVE_URL}"
http_get "$CHECKSUM_URL" > "$CHECKSUM_PATH" || fail "failed to download ${CHECKSUM_URL}"
verify_checksum "$ARCHIVE_PATH" "$CHECKSUM_PATH"

tar -xzf "$ARCHIVE_PATH" -C "$TMP_DIR"
PACKAGE_DIR="${TMP_DIR}/${PACKAGE_NAME}"
[[ -d "$PACKAGE_DIR" ]] || fail "release archive did not unpack ${PACKAGE_NAME}"

IFS=',' read -r -a BIN_LIST <<< "$BINARIES"
for binary in "${BIN_LIST[@]}"; do
  binary="${binary#"${binary%%[![:space:]]*}"}"
  binary="${binary%"${binary##*[![:space:]]}"}"
  [[ -n "$binary" ]] || continue
  install_binary "$PACKAGE_DIR" "$binary"
done

print_path_hint
log
log "Run 'bird transport' to confirm the active transport."
