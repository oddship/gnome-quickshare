#!/usr/bin/env bash
set -euo pipefail
project_dir=$(cd "$(dirname "$0")/../.." && pwd)
binary="${1:?Usage: scripts/i18n/smoke.sh BINARY OUTPUT}"
output="${2:?Choose an evidence directory}"
mkdir -p "$output"
output=$(cd "$output" && pwd)
sandbox=$(mktemp -d -t quickshare-i18n.XXXXXX)
trap 'rm -rf "$sandbox"' EXIT
python3 "$project_dir/scripts/i18n/pseudo.py" "$project_dir/po/gnome-quickshare.pot" "$sandbox/po/zz.po"
printf 'zz\n' > "$sandbox/po/LINGUAS"
mkdir -p "$sandbox/data"
cp "$project_dir/data/io.github.oddship.GnomeQuickshare.desktop" "$sandbox/data/"
cp "$project_dir/data/io.github.oddship.GnomeQuickshare.metainfo.xml" "$sandbox/data/"
(cd "$sandbox" && bash "$project_dir/po/install.sh" "$sandbox/prefix")
cp "$sandbox/prefix/share/applications/io.github.oddship.GnomeQuickshare.desktop" "$output/"
cp "$sandbox/prefix/share/metainfo/io.github.oddship.GnomeQuickshare.metainfo.xml" "$output/"
desktop-file-validate "$output/io.github.oddship.GnomeQuickshare.desktop"
# GNU gettext deliberately disables translation in C/C.UTF-8 locales.
export GNOME_QUICKSHARE_LOCALEDIR="$sandbox/prefix/share/locale" LANGUAGE=zz
export LC_ALL="${QUICKSHARE_TEST_LOCALE:-en_US.UTF-8}"
"$binary" --help > "$output/help.txt"
rg -q '⟦' "$output/help.txt"
rg -q '^GenericName\[zz\]=' "$output/io.github.oddship.GnomeQuickshare.desktop"
rg -q 'xml:lang="zz"' "$output/io.github.oddship.GnomeQuickshare.metainfo.xml"
cd "$project_dir"
bash scripts/ui-smoke.sh "$binary" "$output/ui"
