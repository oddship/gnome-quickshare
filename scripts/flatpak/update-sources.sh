#!/usr/bin/env bash
set -euo pipefail
project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
task_dir=$(mktemp -d -t quickshare-flatpak-sources.XXXXXX)
trap 'rm -rf "$task_dir"' EXIT
revision=9d86d336a39b9af38b86f114b6ce5820fe260f91
curl -fsSL "https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/$revision/cargo/flatpak-cargo-generator.py" -o "$task_dir/generator.py"
printf '%s  %s\n' '0a2db6be87d75910facef28ab46d4d6460802e8419ab850d0caa6a364d26b380' "$task_dir/generator.py" | sha256sum --check --status
cd "$task_dir"
python3 generator.py "$project_dir/Cargo.lock" -o "$project_dir/scripts/flatpak/cargo-sources.json"
python3 "$project_dir/scripts/flatpak/check-sources.py"
