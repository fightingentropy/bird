#!/usr/bin/env python3
"""Verify vendored archives and generate bird's deterministic SPDX SBOM."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import stat
import sys
import tarfile
import tomllib
import zipfile
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parent.parent
DISTFILES = ROOT / "third_party/curl-impersonate/distfiles"
MANIFEST = DISTFILES / "manifest.json"
SBOM = ROOT / "third_party/curl-impersonate/SBOM.spdx.json"
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")


def fail(message: str) -> None:
    raise ValueError(message)


def load_manifest() -> list[dict[str, str]]:
    payload = json.loads(MANIFEST.read_text(encoding="utf-8"))
    entries = payload.get("distfiles")
    if not isinstance(entries, list) or not entries:
        fail("distfile manifest must contain a non-empty distfiles array")
    required = {
        "component",
        "version",
        "license",
        "filename",
        "url",
        "sha256",
        "archive_root",
        "extracted_dir",
    }
    filenames: set[str] = set()
    components: set[str] = set()
    for entry in entries:
        if not isinstance(entry, dict) or not required.issubset(entry):
            fail(f"distfile entry is missing metadata: {entry!r}")
        if entry["filename"] in filenames or entry["component"] in components:
            fail(f"duplicate distfile metadata: {entry['filename']}")
        filenames.add(entry["filename"])
        components.add(entry["component"])
        if not SHA256_RE.fullmatch(entry["sha256"]):
            fail(f"invalid SHA-256 for {entry['filename']}")
    return entries


def safe_archive_path(raw: str, expected_root: str) -> PurePosixPath:
    if not raw or raw.startswith(("/", "\\")) or "\\" in raw:
        fail(f"unsafe archive path: {raw!r}")
    if any(ord(character) < 32 for character in raw):
        fail(f"control character in archive path: {raw!r}")
    path = PurePosixPath(raw)
    if any(part in {"", ".", ".."} for part in path.parts):
        fail(f"archive path traversal: {raw!r}")
    if not path.parts or path.parts[0] != expected_root:
        fail(f"archive path escapes {expected_root!r}: {raw!r}")
    return path


def safe_link_target(link: PurePosixPath, target: str, expected_root: str) -> None:
    if not target or target.startswith(("/", "\\")) or "\\" in target:
        fail(f"unsafe link target: {link} -> {target!r}")
    stack = list(link.parent.parts)
    for part in PurePosixPath(target).parts:
        if part in {"", "."}:
            continue
        if part == "..":
            if len(stack) <= 1:
                fail(f"archive link escapes root: {link} -> {target}")
            stack.pop()
        else:
            stack.append(part)
    if not stack or stack[0] != expected_root:
        fail(f"archive link escapes root: {link} -> {target}")


def inspect_tar(path: Path, expected_root: str) -> None:
    with tarfile.open(path, mode="r:*") as archive:
        members = archive.getmembers()
        if not members:
            fail(f"archive is empty: {path.name}")
        for member in members:
            member_path = safe_archive_path(member.name, expected_root)
            if member.isfile() or member.isdir():
                continue
            if member.issym():
                safe_link_target(member_path, member.linkname, expected_root)
                continue
            fail(f"unsupported tar entry type in {path.name}: {member.name}")


def inspect_zip(path: Path, expected_root: str) -> None:
    with zipfile.ZipFile(path) as archive:
        entries = archive.infolist()
        if not entries:
            fail(f"archive is empty: {path.name}")
        for entry in entries:
            member_path = safe_archive_path(entry.filename.rstrip("/"), expected_root)
            mode = entry.external_attr >> 16
            if stat.S_ISLNK(mode):
                target = archive.read(entry).decode("utf-8")
                safe_link_target(member_path, target, expected_root)
            elif entry.is_dir() or mode == 0 or stat.S_ISREG(mode):
                continue
            else:
                fail(f"unsupported zip entry type in {path.name}: {entry.filename}")


def verify_archives(entries: list[dict[str, str]]) -> None:
    expected_files = {entry["filename"] for entry in entries} | {MANIFEST.name}
    actual_files = {path.name for path in DISTFILES.iterdir() if path.is_file()}
    unexpected = sorted(actual_files - expected_files)
    missing = sorted(expected_files - actual_files)
    if unexpected or missing:
        fail(f"distfile inventory mismatch; missing={missing}, unexpected={unexpected}")

    for entry in entries:
        path = DISTFILES / entry["filename"]
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        if digest != entry["sha256"]:
            fail(
                f"SHA-256 mismatch for {entry['filename']}: "
                f"expected {entry['sha256']}, got {digest}"
            )
        if path.suffix == ".zip":
            inspect_zip(path, entry["archive_root"])
        else:
            inspect_tar(path, entry["archive_root"])


def spdx_id(kind: str, name: str, version: str) -> str:
    safe = re.sub(r"[^A-Za-z0-9.-]", "-", f"{name}-{version}")
    return f"SPDXRef-{kind}-{safe}"


def package_record(
    name: str,
    version: str,
    identifier: str,
    license_name: str = "NOASSERTION",
    checksum: str | None = None,
    download: str = "NOASSERTION",
    purl: str | None = None,
) -> dict[str, object]:
    package: dict[str, object] = {
        "SPDXID": identifier,
        "name": name,
        "versionInfo": version,
        "downloadLocation": download,
        "filesAnalyzed": False,
        "licenseConcluded": license_name,
        "licenseDeclared": license_name,
        "copyrightText": "NOASSERTION",
    }
    if checksum:
        package["checksums"] = [{"algorithm": "SHA256", "checksumValue": checksum}]
    if purl:
        package["externalRefs"] = [
            {
                "referenceCategory": "PACKAGE-MANAGER",
                "referenceType": "purl",
                "referenceLocator": purl,
            }
        ]
    return package


def generate_sbom(entries: list[dict[str, str]]) -> str:
    cargo_lock_text = (ROOT / "Cargo.lock").read_text(encoding="utf-8")
    manifest_text = MANIFEST.read_text(encoding="utf-8")
    cargo = tomllib.loads(cargo_lock_text)
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    version = workspace["workspace"]["package"]["version"]
    root_id = "SPDXRef-Application-bird"
    packages = [
        package_record(
            "bird",
            version,
            root_id,
            "MIT",
            purl=f"pkg:github/fightingentropy/bird@v{version}",
        )
    ]
    relationships: list[dict[str, str]] = []

    seen: set[tuple[str, str]] = set()
    for dependency in sorted(cargo["package"], key=lambda item: (item["name"], item["version"])):
        key = (dependency["name"], dependency["version"])
        if key in seen:
            continue
        seen.add(key)
        identifier = spdx_id("Cargo", *key)
        packages.append(
            package_record(
                dependency["name"],
                dependency["version"],
                identifier,
                checksum=dependency.get("checksum"),
                download=dependency.get("source", "NOASSERTION"),
                purl=f"pkg:cargo/{dependency['name']}@{dependency['version']}",
            )
        )
        relationships.append(
            {
                "spdxElementId": root_id,
                "relationshipType": "DEPENDS_ON",
                "relatedSpdxElement": identifier,
            }
        )

    for entry in sorted(entries, key=lambda item: item["component"]):
        identifier = spdx_id("Vendored", entry["component"], entry["version"])
        packages.append(
            package_record(
                entry["component"],
                entry["version"],
                identifier,
                entry["license"],
                entry["sha256"],
                entry["url"],
                f"pkg:generic/{entry['component']}@{entry['version']}",
            )
        )
        relationships.append(
            {
                "spdxElementId": root_id,
                "relationshipType": "CONTAINS",
                "relatedSpdxElement": identifier,
            }
        )

    source_digest = hashlib.sha256(
        (cargo_lock_text + "\n" + manifest_text).encode("utf-8")
    ).hexdigest()
    document = {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": f"bird-{version}",
        "documentNamespace": (
            f"https://github.com/fightingentropy/bird/sbom/v{version}/{source_digest}"
        ),
        "creationInfo": {
            "created": "2026-08-01T00:00:00Z",
            "creators": ["Tool: scripts/vendor-supply-chain.py"],
            "comment": "Deterministic source SBOM; release provenance supplies the build timestamp.",
        },
        "documentDescribes": [root_id],
        "hasExtractedLicensingInfos": [
            {
                "licenseId": "LicenseRef-BoringSSL",
                "extractedText": (
                    "BoringSSL contains ISC-style and historical OpenSSL/SSLeay terms; "
                    "see LICENSE in the pinned BoringSSL source archive."
                ),
                "name": "BoringSSL bundled license terms",
            }
        ],
        "packages": packages,
        "relationships": relationships,
    }
    return json.dumps(document, indent=2, sort_keys=True) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--write-sbom", action="store_true")
    parser.add_argument("--check-sbom", action="store_true")
    args = parser.parse_args()

    entries = load_manifest()
    verify_archives(entries)
    generated = generate_sbom(entries)
    if args.write_sbom:
        SBOM.write_text(generated, encoding="utf-8")
    if args.check_sbom:
        if not SBOM.is_file() or SBOM.read_text(encoding="utf-8") != generated:
            fail("SBOM is stale; run scripts/vendor-supply-chain.py --write-sbom")
    print(f"verified {len(entries)} vendored archives and their entry paths")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, json.JSONDecodeError, tomllib.TOMLDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
