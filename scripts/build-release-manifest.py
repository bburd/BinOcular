#!/usr/bin/env python3
"""Generate SHA256SUMS and manifest.json for BinOcular release packages."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from datetime import datetime, timezone
from pathlib import Path


IGNORED_NAMES = {"SHA256SUMS", "manifest.json"}


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def build_timestamp() -> str:
    source_date_epoch = os.environ.get("SOURCE_DATE_EPOCH")
    if source_date_epoch:
        timestamp = datetime.fromtimestamp(int(source_date_epoch), tz=timezone.utc)
    else:
        timestamp = datetime.now(timezone.utc)
    return timestamp.replace(microsecond=0).isoformat().replace("+00:00", "Z")


def package_target(name: str, version: str) -> str:
    prefix = f"BinOcular-{version}-"
    if not name.startswith(prefix):
        raise ValueError(f"release package does not start with {prefix!r}: {name}")

    if name.endswith(".tar.gz"):
        return name[len(prefix) : -len(".tar.gz")]
    if name.endswith(".zip"):
        return name[len(prefix) : -len(".zip")]

    raise ValueError(f"unsupported release package extension: {name}")


def target_os(target: str) -> str:
    if "windows" in target:
        return "windows"
    if "linux" in target:
        return "linux"
    return "unknown"


def release_packages(package_dir: Path, version: str) -> list[Path]:
    packages = []
    for path in package_dir.iterdir():
        if path.name in IGNORED_NAMES or not path.is_file():
            continue
        if path.name.startswith(f"BinOcular-{version}-") and (
            path.name.endswith(".zip") or path.name.endswith(".tar.gz")
        ):
            packages.append(path)
    return sorted(packages, key=lambda path: path.name)


def build_manifest(package_dir: Path, version: str, git_commit: str) -> dict[str, object]:
    artifacts = []
    for path in release_packages(package_dir, version):
        target = package_target(path.name, version)
        artifacts.append(
            {
                "name": path.name,
                "target": target,
                "os": target_os(target),
                "arch": target.split("-", 1)[0],
                "size_bytes": path.stat().st_size,
                "sha256": sha256_file(path),
            }
        )

    if not artifacts:
        raise ValueError(f"no release packages found in {package_dir}")

    return {
        "version": version,
        "git_commit": git_commit,
        "build_timestamp": build_timestamp(),
        "artifacts": artifacts,
    }


def write_checksums(manifest: dict[str, object], output_path: Path) -> None:
    artifacts = manifest["artifacts"]
    if not isinstance(artifacts, list):
        raise TypeError("manifest artifacts must be a list")

    lines = [f"{artifact['sha256']}  {artifact['name']}" for artifact in artifacts]
    output_path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Generate BinOcular release manifest and SHA256SUMS."
    )
    parser.add_argument("--package-dir", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--git-commit", required=True)
    parser.add_argument("--output-dir", required=True, type=Path)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    package_dir = args.package_dir.resolve()
    output_dir = args.output_dir.resolve()
    output_dir.mkdir(parents=True, exist_ok=True)

    manifest = build_manifest(package_dir, args.version, args.git_commit)
    write_checksums(manifest, output_dir / "SHA256SUMS")
    (output_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
