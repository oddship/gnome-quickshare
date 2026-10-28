#!/usr/bin/env bash
set -euo pipefail
project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
build_root="${1:-/tmp/gnome-quickshare-flatpak}"
if [[ "$build_root" == / || -z "$build_root" ]]; then echo 'Choose a dedicated build directory.' >&2; exit 2; fi
python3 "$project_dir/scripts/flatpak/check-sources.py"
mkdir -p "$build_root"
build_root=$(cd "$build_root" && pwd)
flatpak-builder --user --disable-rofiles-fuse --force-clean --state-dir="$build_root/state" --repo="$build_root/repo" \
    "$build_root/app" "$project_dir/scripts/flatpak/io.github.oddship.GnomeQuickshare.json"
flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
    "$build_root/repo" "$build_root/gnome-quickshare-0.1.0.flatpak" io.github.oddship.GnomeQuickshare devel
