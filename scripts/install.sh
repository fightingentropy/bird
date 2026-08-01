#!/usr/bin/env bash

set -euo pipefail

REPO="${BIRD_GITHUB_REPO:-fightingentropy/bird}"
INSTALL_DIR="${BIRD_INSTALL_DIR:-$HOME/.local/bin}"
RELEASE_BASE_URL="${BIRD_RELEASE_BASE_URL:-}"
VERSION="${BIRD_VERSION:-}"
BINARIES="${BIRD_BINARIES:-bird,sweet-cookie-diagnose}"
VERIFY_ATTESTATION="${BIRD_VERIFY_ATTESTATION:-0}"

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
  local archive_path checksum_path expected checksum_filename extra actual
  archive_path="$1"
  checksum_path="$2"
  [[ "$(awk 'END { print NR }' "$checksum_path")" == "1" ]] \
    || fail "checksum file must contain exactly one line"
  read -r expected checksum_filename extra < "$checksum_path" || fail "checksum file is empty: $checksum_path"
  checksum_filename="${checksum_filename#\*}"
  [[ "$expected" =~ ^[0-9a-fA-F]{64}$ ]] || fail "checksum file has an invalid SHA-256 digest"
  [[ "$checksum_filename" == "$(basename "$archive_path")" ]] \
    || fail "checksum file names an unexpected archive: ${checksum_filename:-missing}"
  [[ -z "${extra:-}" ]] || fail "checksum file contains unexpected fields"
  actual="$(compute_sha256 "$archive_path")"
  expected="$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')"
  actual="$(printf '%s' "$actual" | tr '[:upper:]' '[:lower:]')"
  [[ "$expected" == "$actual" ]] || fail "checksum mismatch for $(basename "$archive_path")"
}

validate_archive_entries() {
  local archive_path package_name listing verbose entry component type
  archive_path="$1"
  package_name="$2"
  listing="${TMP_DIR}/archive-entries.txt"
  verbose="${TMP_DIR}/archive-entries.verbose.txt"

  tar -tzf "$archive_path" > "$listing" || fail "could not list release archive"
  tar -tvzf "$archive_path" > "$verbose" || fail "could not inspect release archive entry types"
  [[ -s "$listing" ]] || fail "release archive is empty"

  while IFS= read -r entry || [[ -n "$entry" ]]; do
    [[ -n "$entry" ]] || fail "release archive contains an empty entry name"
    [[ "$entry" != /* && "$entry" != *\\* ]] \
      || fail "release archive contains an unsafe path: $entry"
    IFS='/' read -r -a components <<< "$entry"
    for component in "${components[@]}"; do
      [[ "$component" != ".." && "$component" != "." && -n "$component" ]] \
        || [[ "$component" == "" && "$entry" == */ ]] \
        || fail "release archive contains path traversal: $entry"
    done
    case "$entry" in
      "$package_name"/|"$package_name"/bin/|"$package_name"/bin/bird|\
      "$package_name"/bin/sweet-cookie-diagnose|"$package_name"/README.md|\
      "$package_name"/BUILD-INFO.txt|"$package_name"/SBOM.spdx.json)
        ;;
      *)
        fail "release archive contains an unexpected entry: $entry"
        ;;
    esac
  done < "$listing"

  [[ -z "$(sort "$listing" | uniq -d)" ]] || fail "release archive contains duplicate entries"
  for entry in \
    "$package_name/bin/bird" \
    "$package_name/bin/sweet-cookie-diagnose" \
    "$package_name/README.md" \
    "$package_name/BUILD-INFO.txt" \
    "$package_name/SBOM.spdx.json"; do
    grep -Fxq "$entry" "$listing" || fail "release archive is missing $entry"
  done

  while IFS= read -r entry || [[ -n "$entry" ]]; do
    type="${entry:0:1}"
    [[ "$type" == "-" || "$type" == "d" ]] \
      || fail "release archive contains a link or special file"
  done < "$verbose"
}

verify_attestation() {
  local archive_path="$1"
  [[ "$VERIFY_ATTESTATION" == "1" ]] || return 0
  [[ -z "$RELEASE_BASE_URL" ]] \
    || fail "attestation verification is available only for GitHub-hosted releases"
  require_tool gh
  gh attestation verify "$archive_path" --repo "$REPO" \
    || fail "GitHub artifact attestation verification failed"
  log "verified GitHub artifact attestation for $(basename "$archive_path")"
}

install_binary() {
  local package_dir binary source target
  package_dir="$1"
  binary="$2"
  source="${package_dir}/bin/${binary}"
  target="${INSTALL_DIR}/${binary}"
  [[ -f "$source" && ! -L "$source" ]] || fail "release archive is missing a regular ${binary}"
  install -m 755 "$source" "$target"
  log "installed ${target}"
}

verify_bird_version() {
  local binary_path label version_output installed_version
  binary_path="$1"
  label="$2"
  [[ -x "$binary_path" ]] || fail "${label} is missing or not executable: ${binary_path}"
  if ! version_output="$("$binary_path" --version 2>&1)"; then
    fail "could not run ${label}: ${binary_path}"
  fi
  installed_version="$(awk 'NR==1 { print $2 }' <<< "$version_output")"
  [[ "$installed_version" == "$RELEASE_VERSION" ]] \
    || fail "${label} reports bird version ${installed_version:-unknown}; expected ${RELEASE_VERSION}"
  log "verified ${label} ${installed_version}"
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

if [[ "${BIRD_TEST_VALIDATE_ARCHIVE:-}" == "1" ]]; then
  [[ $# -eq 2 ]] || fail "archive validation test mode expects ARCHIVE PACKAGE_NAME"
  TMP_DIR="$(mktemp -d)"
  chmod 700 "$TMP_DIR"
  validate_archive_entries "$1" "$2"
  exit 0
fi

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

RELEASE_TAG="v${VERSION#v}"
RELEASE_VERSION="${VERSION#v}"
PACKAGE_NAME="bird-v${RELEASE_VERSION}-${TARGET}"
ARCHIVE_NAME="${PACKAGE_NAME}.tar.gz"
CHECKSUM_NAME="${ARCHIVE_NAME}.sha256"

if [[ -n "$RELEASE_BASE_URL" ]]; then
  ARCHIVE_URL="${RELEASE_BASE_URL%/}/${ARCHIVE_NAME}"
  CHECKSUM_URL="${RELEASE_BASE_URL%/}/${CHECKSUM_NAME}"
else
  ARCHIVE_URL="https://github.com/${REPO}/releases/download/${RELEASE_TAG}/${ARCHIVE_NAME}"
  CHECKSUM_URL="https://github.com/${REPO}/releases/download/${RELEASE_TAG}/${CHECKSUM_NAME}"
fi

TMP_DIR="$(mktemp -d)"
chmod 700 "$TMP_DIR"
ARCHIVE_PATH="${TMP_DIR}/${ARCHIVE_NAME}"
CHECKSUM_PATH="${TMP_DIR}/${CHECKSUM_NAME}"

log "Installing ${PACKAGE_NAME} from ${REPO}"
mkdir -p "$INSTALL_DIR"
[[ -w "$INSTALL_DIR" ]] || fail "install directory is not writable: ${INSTALL_DIR}"

http_get "$ARCHIVE_URL" > "$ARCHIVE_PATH" || fail "failed to download ${ARCHIVE_URL}"
http_get "$CHECKSUM_URL" > "$CHECKSUM_PATH" || fail "failed to download ${CHECKSUM_URL}"
verify_checksum "$ARCHIVE_PATH" "$CHECKSUM_PATH"
verify_attestation "$ARCHIVE_PATH"

validate_archive_entries "$ARCHIVE_PATH" "$PACKAGE_NAME"
tar -xzf "$ARCHIVE_PATH" -C "$TMP_DIR"
PACKAGE_DIR="${TMP_DIR}/${PACKAGE_NAME}"
[[ -d "$PACKAGE_DIR" && ! -L "$PACKAGE_DIR" ]] || fail "release archive did not unpack a regular ${PACKAGE_NAME} directory"

IFS=',' read -r -a BIN_LIST <<< "$BINARIES"
INSTALLED_BIRD=0
for binary in "${BIN_LIST[@]}"; do
  binary="${binary#"${binary%%[![:space:]]*}"}"
  binary="${binary%"${binary##*[![:space:]]}"}"
  [[ -n "$binary" ]] || continue
  if [[ "$binary" == "bird" ]]; then
    verify_bird_version "${PACKAGE_DIR}/bin/bird" "release package"
  fi
  install_binary "$PACKAGE_DIR" "$binary"
  if [[ "$binary" == "bird" ]]; then
    INSTALLED_BIRD=1
  fi
done

if [[ "$INSTALLED_BIRD" == "1" ]]; then
  verify_bird_version "${INSTALL_DIR}/bird" "installed bird"
fi

print_path_hint
log
log "Run 'bird transport' to confirm the active transport."
