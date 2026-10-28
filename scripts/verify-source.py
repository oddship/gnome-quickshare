#!/usr/bin/env python3
"""Compare application source/resource inputs with a package build snapshot."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

EXPLICIT = (
    "Cargo.toml", "Cargo.lock", "vendor/rqs_lib/Cargo.toml", "vendor/rqs_lib/build.rs",
    "LICENSE", "vendor/rqs_lib/LICENSE", "vendor/rqs_lib/LICENSE-APACHE",
    "vendor/rqs_lib/LICENSE-CHROMIUM-BSD", "po/install.sh", "po/LINGUAS",
    "po/gnome-quickshare.pot", "po/POTFILES.in",
)


def inputs(root):
    paths = {Path(name) for name in EXPLICIT}
    for directory in ["src", "tests", "vendor/rqs_lib/src"]:
        paths.update(p.relative_to(root) for p in (root / directory).rglob("*.rs"))
    paths.update(p.relative_to(root) for p in (root / "vendor/rqs_lib/src").rglob("*.proto"))
    paths.update(p.relative_to(root) for p in (root / "data").rglob("*") if p.is_file())
    paths.update(p.relative_to(root) for p in (root / "po").glob("*.po"))
    return paths


def digest(path):
    if path.is_symlink():
        raise ValueError(f"Source input must not be a symlink: {path}")
    if not path.is_file():
        return None
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path)
    parser.add_argument("snapshot", type=Path)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--retain", type=Path)
    args = parser.parse_args()
    reference = args.reference.resolve(strict=True)
    snapshot = args.snapshot.resolve(strict=True)
    files = []
    for name in sorted(inputs(reference) | inputs(snapshot)):
        expected, actual = digest(reference / name), digest(snapshot / name)
        files.append({"file": str(name), "match": expected is not None and expected == actual,
                      "referenceSha256": expected, "sha256": actual})
    result = {"referenceRoot": str(reference), "sourceSnapshot": str(snapshot),
              "allMatch": all(item["match"] for item in files), "files": files}
    if args.retain and result["allMatch"]:
        retained = args.retain.absolute()
        for item in files:
            destination = retained / item["file"]
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(snapshot / item["file"], destination)
        result["retainedSnapshot"] = str(retained)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(result, indent=2) + "\n")
    if not result["allMatch"]:
        mismatches = [item["file"] for item in files if not item["match"]]
        raise SystemExit("Source mismatch: " + ", ".join(mismatches))
    print(f"All {len(files)} application source/resource inputs match.")


if __name__ == "__main__":
    main()
