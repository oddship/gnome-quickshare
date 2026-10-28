#!/usr/bin/env bash
set -euo pipefail
# The smoke drivers supply isolated XDG directories. Keep Flatpak's own runtime
# outside those directories, and forward the app paths after its XDG setup.
: "${QUICKSHARE_FLATPAK_RUNTIME_DIR:?Set to the original Flatpak XDG_RUNTIME_DIR}"
: "${XDG_CONFIG_HOME:?Run through a smoke driver}"
test_root=$(dirname "$XDG_CONFIG_HOME")
options=(--user --filesystem="$test_root" --own-name=io.github.oddship.GnomeQuickshare)
if [[ "${QUICKSHARE_FLATPAK_LIVE:-0}" == 1 ]]; then
    : "${WAYLAND_DISPLAY:?A live Wayland compositor is required}"
    if [[ "$WAYLAND_DISPLAY" != /* ]]; then export WAYLAND_DISPLAY="$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"; fi
    options+=(--socket=wayland --nosocket=x11 --nosocket=fallback-x11 --env=GDK_BACKEND=wayland --env=GTK_A11Y=atspi)
else
    options+=(--no-documents-portal --no-a11y-bus --socket=x11 --env=GDK_BACKEND=x11 --env=GTK_A11Y=none --env=GSETTINGS_BACKEND=memory --env=GIO_USE_VFS=local)
fi
# Forward only the two explicit theme-debug knobs used by isolated captures.
for theme_option in ADW_DEBUG_COLOR_SCHEME ADW_DEBUG_HIGH_CONTRAST; do
    if [[ -n "${!theme_option:-}" ]]; then options+=("--env=$theme_option=${!theme_option}"); fi
done
exec env XDG_RUNTIME_DIR="$QUICKSHARE_FLATPAK_RUNTIME_DIR" \
    flatpak run "${options[@]}" \
    --env=SMOKE_CONFIG_HOME="$XDG_CONFIG_HOME" \
    --env=SMOKE_STATE_HOME="$XDG_STATE_HOME" \
    --env=SMOKE_DATA_HOME="$XDG_DATA_HOME" \
    --env=SMOKE_CACHE_HOME="$XDG_CACHE_HOME" \
    --env=DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SYSTEM_BUS_ADDRESS" \
    --env=MESA_SHADER_CACHE_DISABLE=true --env=GSK_RENDERER=cairo \
    --command=sh io.github.oddship.GnomeQuickshare \
    -c 'export XDG_CONFIG_HOME="$SMOKE_CONFIG_HOME" XDG_STATE_HOME="$SMOKE_STATE_HOME" XDG_DATA_HOME="$SMOKE_DATA_HOME" XDG_CACHE_HOME="$SMOKE_CACHE_HOME"; exec /app/bin/gnome-quickshare "$@"' sh "$@"
