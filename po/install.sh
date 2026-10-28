#!/usr/bin/env bash
set -euo pipefail
prefix="${1:?Usage: bash po/install.sh PREFIX}"
mkdir -p "$prefix/share/applications" "$prefix/share/metainfo"
for language in $(sed '/^[[:space:]]*#/d' po/LINGUAS); do
    [[ "$language" =~ ^[a-zA-Z0-9_@.-]+$ ]] || { echo 'Invalid language name' >&2; exit 1; }
    mkdir -p "$prefix/share/locale/$language/LC_MESSAGES"
    msgfmt --check --check-format "po/$language.po" -o "$prefix/share/locale/$language/LC_MESSAGES/gnome-quickshare.mo"
done
if [[ -n "$(sed '/^[[:space:]]*#/d; /^[[:space:]]*$/d' po/LINGUAS)" ]]; then
    msgfmt --desktop --template=data/io.github.oddship.GnomeQuickshare.desktop -d po \
        --output="$prefix/share/applications/io.github.oddship.GnomeQuickshare.desktop"
    its="${GETTEXT_METAINFO_ITS:-$(dirname "$(dirname "$(command -v appstreamcli)")")/share/gettext/its/metainfo.its}"
    GETTEXTDATADIRS="$(dirname "$(dirname "$its")")${GETTEXTDATADIRS:+:$GETTEXTDATADIRS}" \
    msgfmt --xml --language=MetaInfo --template=data/io.github.oddship.GnomeQuickshare.metainfo.xml -d po \
        --output="$prefix/share/metainfo/io.github.oddship.GnomeQuickshare.metainfo.xml"
else
    install -m644 data/io.github.oddship.GnomeQuickshare.desktop "$prefix/share/applications/"
    install -m644 data/io.github.oddship.GnomeQuickshare.metainfo.xml "$prefix/share/metainfo/"
fi
