#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
xgettext --language=Rust --from-code=UTF-8 --sort-output --no-wrap \
    --keyword= --keyword=tr:1 --keyword=trf:1 --keyword=ntr:1,2 --keyword=mark:1 \
    --keyword=label:1 --keyword=section:1 --package-name=gnome-quickshare \
    --package-version=0.1.0 --files-from=po/POTFILES.in --output=po/gnome-quickshare.pot
xgettext --language=Desktop --from-code=UTF-8 --join-existing --sort-output --no-wrap \
    --output=po/gnome-quickshare.pot data/io.github.oddship.GnomeQuickshare.desktop
its="${GETTEXT_METAINFO_ITS:-$(dirname "$(dirname "$(command -v appstreamcli)")")/share/gettext/its/metainfo.its}"
xgettext --its="$its" --from-code=UTF-8 --join-existing --sort-output --no-wrap \
    --output=po/gnome-quickshare.pot data/io.github.oddship.GnomeQuickshare.metainfo.xml
while read -r language; do
    [[ -z "$language" || "$language" == \#* ]] && continue
    msgmerge --update --backup=none --no-wrap "po/$language.po" po/gnome-quickshare.pot
done < po/LINGUAS
