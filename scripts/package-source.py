#!/usr/bin/env python3
"""Create a deterministic development source archive from explicit project paths."""
import argparse
import gzip
import hashlib
import os
from pathlib import Path
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]
PREFIX = "gnome-quickshare-0.1.0"
INCLUDE = (
    "Cargo.toml", "Cargo.lock", "LICENSE", "README.md", "flake.nix", "flake.lock",
    "justfile", ".gitignore", ".github", "data", "src", "vendor", "tests",
    "scripts", "po", "extension", "docs",
)
SKIP = {".git", ".aws", ".agents", ".codex", "__pycache__", "target", ".flatpak-builder"}


def source_files():
    for name in INCLUDE:
        path = ROOT / name
        if not path.exists():
            continue
        candidates = [path, *sorted(path.rglob("*"))] if path.is_dir() else [path]
        for candidate in candidates:
            relative = candidate.relative_to(ROOT)
            if SKIP.intersection(relative.parts) or candidate.suffix in {".pyc", ".mo"}:
                continue
            if candidate.is_symlink():
                raise ValueError(f"Source archive refuses symlinks: {relative}")
            if candidate.is_dir() or candidate.is_file():
                yield candidate, relative


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="Destination .tar.gz file")
    args = parser.parse_args()
    output = args.output.absolute()
    if not output.name.endswith(".tar.gz"):
        parser.error("Output must end in .tar.gz")
    if output.is_relative_to(ROOT):
        relative = output.relative_to(ROOT)
        if relative.parts[0] in INCLUDE:
            parser.error("Output cannot be inside an archived source path")
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    count = 0
    try:
        with tempfile.NamedTemporaryFile(dir=output.parent, delete=False) as raw:
            temporary = Path(raw.name)
            with gzip.GzipFile(filename="", fileobj=raw, mode="wb", mtime=0) as compressed:
                with tarfile.open(fileobj=compressed, mode="w|") as archive:
                    for path, relative in source_files():
                        info = archive.gettarinfo(str(path), arcname=f"{PREFIX}/{relative}")
                        info.uid = info.gid = info.mtime = 0
                        info.uname = info.gname = ""
                        info.mode = 0o755 if path.is_dir() or info.mode & 0o111 else 0o644
                        if path.is_file():
                            with path.open("rb") as source:
                                archive.addfile(info, source)
                            count += 1
                        else:
                            archive.addfile(info)
        os.replace(temporary, output)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    print(f"{output}: {count} source files")
    with output.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    print(f"SHA256 {digest}")


if __name__ == "__main__":
    main()
