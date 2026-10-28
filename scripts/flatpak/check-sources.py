#!/usr/bin/env python3
"""Reject a stale Flatpak dependency list before starting a lengthy SDK build."""
import json
from pathlib import Path
import tomllib
from urllib.parse import urlparse

ROOT = Path(__file__).resolve().parents[2]


def canonical_git(value):
    parsed = urlparse(value.removeprefix("git+"))
    return (parsed.hostname, parsed.path.lower().removesuffix(".git").rstrip("/"))


def main():
    packages = tomllib.loads((ROOT / "Cargo.lock").read_text())["package"]
    sources = json.loads((ROOT / "scripts/flatpak/cargo-sources.json").read_text())
    archives = {source["url"]: source["sha256"] for source in sources if source["type"] == "archive"}
    git_sources = {(canonical_git(source["url"]), source["commit"]) for source in sources if source["type"] == "git"}
    expected_archives = {}
    expected_git = set()
    for package in packages:
        source = package.get("source", "")
        if source.startswith("registry+"):
            url = f'https://static.crates.io/crates/{package["name"]}/{package["name"]}-{package["version"]}.crate'
            expected_archives[url] = package["checksum"]
        elif source.startswith("git+"):
            expected_git.add((canonical_git(source), source.rsplit("#", 1)[1]))
        elif source:
            raise SystemExit(f"Unsupported Cargo source: {source}")
    if archives != expected_archives or git_sources != expected_git:
        raise SystemExit("Flatpak sources differ from Cargo.lock. Run scripts/flatpak/update-sources.sh.")
    print(f"Flatpak sources match Cargo.lock: {len(archives)} crate archives, {len(git_sources)} Git revisions.")


if __name__ == "__main__":
    main()
